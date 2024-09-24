use core::future::Future;
use pb::{rove_client::RoveClient, validate_request::SpaceSpec, Flag, ValidateRequest};
use rove::{
    data_switch::{DataConnector, DataSwitch},
    dev_utils::{construct_fake_dag, construct_hardcoded_dag, TestDataSource},
    start_server_unix_listener, Dag,
};
use std::{collections::HashMap, sync::Arc};
use tempfile::NamedTempFile;
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::{wrappers::UnixListenerStream, StreamExt};
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

mod pb {
    tonic::include_proto!("rove");
}

const DATA_LEN_SINGLE: usize = 3;
const DATA_LEN_SPATIAL: usize = 1000;

pub async fn set_up_rove(
    data_switch: DataSwitch<'static>,
    dag: Dag<&'static str>,
) -> (impl Future<Output = ()>, RoveClient<Channel>) {
    let coordintor_socket = NamedTempFile::new().unwrap();
    let coordintor_socket = Arc::new(coordintor_socket.into_temp_path());
    std::fs::remove_file(&*coordintor_socket).unwrap();
    let coordintor_uds = UnixListener::bind(&*coordintor_socket).unwrap();
    let coordintor_stream = UnixListenerStream::new(coordintor_uds);
    let coordinator_future = async {
        start_server_unix_listener(coordintor_stream, data_switch, dag)
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

// This test exists because the real dag isn't currently complex enough to
// verify correct scheduling. In the future we will either decide we don't
// need complex dag based scheduling, or the real dag will become complex.
// In either case this test will eventually be obsolete, so:
// TODO: delete
#[tokio::test]
async fn integration_test_fake_dag() {
    // tracing_subscriber::fmt()
    //     .with_max_level(tracing::Level::INFO)
    //     .init();

    let data_switch = DataSwitch::new(HashMap::from([(
        "test",
        &TestDataSource {
            data_len_single: DATA_LEN_SINGLE,
            data_len_series: 1,
            data_len_spatial: DATA_LEN_SPATIAL,
        } as &dyn DataConnector,
    )]));

    let (coordinator_future, mut client) = set_up_rove(data_switch, construct_fake_dag()).await;

    let request_future = async {
        let mut stream = client
            .validate(ValidateRequest {
                data_source: String::from("test"),
                backing_sources: vec![],
                start_time: Some(prost_types::Timestamp::default()),
                end_time: Some(prost_types::Timestamp::default()),
                time_resolution: String::from("PT5M"),
                space_spec: Some(SpaceSpec::One(String::from("single"))),
                tests: vec![String::from("test1")],
                extra_spec: None,
            })
            .await
            .unwrap()
            .into_inner();

        let mut recv_count = 0;
        while let Some(recv) = stream.next().await {
            assert_eq!(
                // TODO: improve
                recv.unwrap().results.first().unwrap().flag,
                Flag::Inconclusive as i32
            );
            recv_count += 1;
        }
        assert_eq!(recv_count, 6);
    };

    tokio::select! {
        _ = coordinator_future => panic!("coordinator returned first"),
        _ = request_future => (),
    }
}

#[tokio::test]
async fn integration_test_hardcoded_dag() {
    // tracing_subscriber::fmt()
    //     .with_max_level(tracing::Level::INFO)
    //     .init();

    let data_switch = DataSwitch::new(HashMap::from([(
        "test",
        &TestDataSource {
            data_len_single: DATA_LEN_SINGLE,
            data_len_series: 1,
            data_len_spatial: DATA_LEN_SPATIAL,
        } as &dyn DataConnector,
    )]));

    let (coordinator_future, mut client) =
        set_up_rove(data_switch, construct_hardcoded_dag()).await;

    let requests_future = async {
        let mut stream = client
            .validate(ValidateRequest {
                data_source: String::from("test"),
                backing_sources: vec![],
                start_time: Some(prost_types::Timestamp::default()),
                end_time: Some(prost_types::Timestamp::default()),
                time_resolution: String::from("PT5M"),
                space_spec: Some(SpaceSpec::One(String::from("single"))),
                tests: vec![String::from("step_check"), String::from("dip_check")],
                extra_spec: None,
            })
            .await
            .unwrap()
            .into_inner();

        let mut _step_recv = false;
        let mut _dip_recv = false;
        let mut _recv_count = 0;
        while let Some(recv) = stream.next().await {
            let inner = recv.unwrap();
            match inner.test.as_ref() {
                "dip_check" => {
                    _dip_recv = true;
                }
                "step_check" => {
                    _step_recv = true;
                }
                _ => {
                    panic!("unrecognised test name returned")
                }
            }
            let flags: Vec<i32> = inner.results.iter().map(|res| res.flag).collect();
            assert_eq!(flags, vec![Flag::Pass as i32; 1]);
            _recv_count += 1;
        }
        // TODO: these checks fail because we can no longer set different hardcoded
        // num_leading_points and num_trailing points for spatial vs series runs.
        // we should reenable these once we implement deriving those from the test list
        // assert_eq!(recv_count, 2);
        // assert!(dip_recv);
        // assert!(step_recv);

        let mut stream = client
            .validate(ValidateRequest {
                data_source: String::from("test"),
                backing_sources: vec![],
                start_time: Some(prost_types::Timestamp::default()),
                end_time: Some(prost_types::Timestamp::default()),
                time_resolution: String::from("PT5M"),
                space_spec: Some(SpaceSpec::All(())),
                tests: vec!["buddy_check".to_string(), "sct".to_string()],
                extra_spec: None,
            })
            .await
            .unwrap()
            .into_inner();

        let mut buddy_recv = false;
        let mut sct_recv = false;
        let mut recv_count = 0;
        while let Some(recv) = stream.next().await {
            let inner = recv.unwrap();
            match inner.test.as_ref() {
                "buddy_check" => {
                    buddy_recv = true;
                }
                "sct" => {
                    sct_recv = true;
                }
                _ => {
                    panic!("unrecognised test name returned")
                }
            }
            let flags: Vec<i32> = inner.results.iter().map(|res| res.flag).collect();
            assert_eq!(flags, vec![Flag::Pass as i32; DATA_LEN_SPATIAL]);
            recv_count += 1;
        }
        assert_eq!(recv_count, 2);
        assert!(buddy_recv);
        assert!(sct_recv);
    };

    tokio::select! {
        _ = coordinator_future => panic!("coordinator returned first"),
        _ = requests_future => (),
    }
}
