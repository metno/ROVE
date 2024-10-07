use crate::{
    data_switch::{DataSwitch, GeoPoint, SpaceSpec, TimeSpec, Timerange, Timestamp},
    pb::{
        self,
        rove_server::{Rove, RoveServer},
        ValidateRequest, ValidateResponse,
    },
    pipeline::Pipeline,
    scheduler::{self, Scheduler},
};
use chronoutil::RelativeDuration;
use futures::Stream;
use std::{collections::HashMap, net::SocketAddr, pin::Pin};
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::{transport::Server, Request, Response, Status};

type ResponseStream = Pin<Box<dyn Stream<Item = Result<ValidateResponse, Status>> + Send>>;

#[derive(Debug)]
enum ListenerType {
    Addr(SocketAddr),
    UnixListener(UnixListenerStream),
}

impl From<scheduler::Error> for Status {
    fn from(item: scheduler::Error) -> Self {
        match item {
            scheduler::Error::InvalidArg(s) => {
                Status::invalid_argument(format!("invalid argument: {}", s))
            }
            scheduler::Error::Runner(e) => Status::aborted(format!("failed to run test: {}", e)),
            scheduler::Error::DataSwitch(e) => {
                Status::not_found(format!("data switch failed to find data: {}", e))
            }
        }
    }
}

#[tonic::async_trait]
impl Rove for Scheduler<'static> {
    type ValidateStream = ResponseStream;

    #[tracing::instrument]
    async fn validate(
        &self,
        request: Request<ValidateRequest>,
    ) -> Result<Response<Self::ValidateStream>, Status> {
        tracing::debug!("Got a request: {:?}", request);

        let req = request.into_inner();

        let time_spec = TimeSpec {
            timerange: Timerange {
                start: Timestamp(
                    req.start_time
                        .as_ref()
                        .ok_or(Status::invalid_argument("invalid timestamp for start_time"))?
                        .seconds,
                ),
                end: Timestamp(
                    req.end_time
                        .as_ref()
                        .ok_or(Status::invalid_argument("invalid timestamp for start_time"))?
                        .seconds,
                ),
            },
            time_resolution: RelativeDuration::parse_from_iso8601(&req.time_resolution)
                .map_err(|e| Status::invalid_argument(format!("invalid time_resolution: {}", e)))?,
        };

        // TODO: implementing From<pb::validate_request::SpaceSpec> for SpaceSpec
        // would make this much neater
        let space_spec = match req.space_spec.unwrap() {
            pb::validate_request::SpaceSpec::One(station_id) => SpaceSpec::One(station_id),
            pb::validate_request::SpaceSpec::Polygon(pb_polygon) => SpaceSpec::Polygon(
                pb_polygon
                    .polygon
                    .into_iter()
                    .map(|point| GeoPoint {
                        lat: point.lat,
                        lon: point.lon,
                    })
                    .collect::<Vec<GeoPoint>>(),
            ),
            pb::validate_request::SpaceSpec::All(_) => SpaceSpec::All,
        };

        let mut rx = self
            .validate_direct(
                req.data_source,
                &req.backing_sources,
                &time_spec,
                &space_spec,
                &req.pipeline,
                req.extra_spec.as_deref(),
            )
            .await
            .map_err(Into::<Status>::into)?;

        // this unwrap is fine because validate_direct already checked the hashmap entry exists
        let pipeline_len = self.pipelines.get(&req.pipeline).unwrap().steps.len();

        // TODO: remove this channel chaining once async iterators drop
        let (tx_final, rx_final) = channel(pipeline_len);
        tokio::spawn(async move {
            while let Some(i) = rx.recv().await {
                match tx_final.send(i.map_err(|e| e.into())).await {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                };
            }
        });

        let output_stream = ReceiverStream::new(rx_final);
        Ok(Response::new(
            Box::pin(output_stream) as Self::ValidateStream
        ))
    }
}

async fn start_server_inner(
    listener: ListenerType,
    data_switch: DataSwitch<'static>,
    pipelines: HashMap<String, Pipeline>,
) -> Result<(), Box<dyn std::error::Error>> {
    let rove_service = Scheduler::new(pipelines, data_switch);

    match listener {
        ListenerType::Addr(addr) => {
            tracing::info!(message = "Starting server.", %addr);

            Server::builder()
                .trace_fn(|_| tracing::info_span!("helloworld_server"))
                .add_service(RoveServer::new(rove_service))
                .serve(addr)
                .await?;
        }
        ListenerType::UnixListener(stream) => {
            Server::builder()
                .add_service(RoveServer::new(rove_service))
                .serve_with_incoming(stream)
                .await?;
        }
    }

    Ok(())
}

/// Equivalent to `start_server`, but using a unix listener instead of listening
/// on a socket, to enable more deterministic integration testing.
#[doc(hidden)]
pub async fn start_server_unix_listener(
    stream: UnixListenerStream,
    data_switch: DataSwitch<'static>,
    pipelines: HashMap<String, Pipeline>,
) -> Result<(), Box<dyn std::error::Error>> {
    start_server_inner(ListenerType::UnixListener(stream), data_switch, pipelines).await
}

/// Starts up a gRPC server to process QC run requests
///
/// Takes a [socket address](std::net::SocketAddr) to listen on, a
/// [data switch](DataSwitch) to provide access to data sources, and a hashmap
/// of pipelines of checks that can be run on data, keyed by their names.
pub async fn start_server(
    addr: SocketAddr,
    data_switch: DataSwitch<'static>,
    pipelines: HashMap<String, Pipeline>,
) -> Result<(), Box<dyn std::error::Error>> {
    start_server_inner(ListenerType::Addr(addr), data_switch, pipelines).await
}
