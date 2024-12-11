use core::future::Future;
use pb::{rove_client::RoveClient, validate_request::SpaceSpec, Flag, ValidateRequest};
use rove::{
    data_switch::{DataConnector, DataSwitch},
    dev_utils::{construct_hardcoded_pipeline, TestDataSource},
    start_server_unix_listener, Pipeline,
};
use std::{collections::HashMap, sync::Arc};
use tempfile::NamedTempFile;
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

mod pb {
    tonic::include_proto!("rove");
}

const DATA_LEN_SINGLE: usize = 3;
const DATA_LEN_SPATIAL: usize = 1000;

pub async fn set_up_rove(
    data_switch: DataSwitch,
    pipelines: HashMap<String, Pipeline>,
) -> (impl Future<Output = ()>, RoveClient<Channel>) {
    let coordintor_socket = NamedTempFile::new().unwrap();
    let coordintor_socket = Arc::new(coordintor_socket.into_temp_path());
    std::fs::remove_file(&*coordintor_socket).unwrap();
    let coordintor_uds = UnixListener::bind(&*coordintor_socket).unwrap();
    let coordintor_stream = UnixListenerStream::new(coordintor_uds);
    let coordinator_future = async {
        start_server_unix_listener(coordintor_stream, data_switch, pipelines)
            .await
            .unwrap();
    };

    let coordinator_channel = Endpoint::try_from("http://any.url")
        .unwrap()
        .connect_with_connector(service_fn(move |_: tonic::transport::Uri| {
            let socket = Arc::clone(&coordintor_socket);
            async move { UnixStream::connect(&*socket).await }
        }))
        .await
        .unwrap();
    let client = RoveClient::new(coordinator_channel);

    (coordinator_future, client)
}

// TODO: we should probably just use one of the sample pipelines here once we have the checks
// from olympian working
#[tokio::test]
async fn integration_test_hardcoded_pipeline() {
    let data_switch = DataSwitch::new(HashMap::from([(
        String::from("test"),
        Box::new(TestDataSource {
            data_len_single: DATA_LEN_SINGLE,
            data_len_series: 1,
            data_len_spatial: DATA_LEN_SPATIAL,
        }) as Box<dyn DataConnector + Send>,
    )]));

    let (coordinator_future, mut client) =
        set_up_rove(data_switch, construct_hardcoded_pipeline()).await;

    let requests_future = async {
        let response = client
            .validate(ValidateRequest {
                data_source: String::from("test"),
                backing_sources: vec![],
                start_time: Some(prost_types::Timestamp::default()),
                end_time: Some(prost_types::Timestamp::default()),
                time_resolution: String::from("PT5M"),
                space_spec: Some(SpaceSpec::All(())),
                pipeline: String::from("hardcoded"),
                extra_spec: None,
            })
            .await
            .unwrap()
            .into_inner();

        assert_eq!(response.results.len(), 4);
        assert!(response.results.iter().any(|c| c.check == "step_check"));
        assert!(response.results.iter().any(|c| c.check == "spike_check"));
        assert!(response.results.iter().any(|c| c.check == "buddy_check"));
        assert!(response.results.iter().any(|c| c.check == "sct"));
        println!("{:?}", response);
        assert!(response.results.iter().all(|c| {
            let flags: Vec<i32> = c
                .flag_series
                .iter()
                .map(|fs| *fs.flags.first().unwrap())
                .collect();
            flags == vec![Flag::Pass as i32; DATA_LEN_SPATIAL]
                || flags == vec![Flag::Isolated as i32; DATA_LEN_SPATIAL]
        }))
    };

    tokio::select! {
        _ = coordinator_future => panic!("coordinator returned first"),
        _ = requests_future => (),
    }
}
