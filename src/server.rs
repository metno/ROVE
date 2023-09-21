use crate::{
    data_switch::{DataSwitch, GeoPoint, SeriesCache, SpatialCache, Timerange, Timestamp},
    harness,
    pb::{
        rove_server::{Rove, RoveServer},
        ValidateSeriesRequest, ValidateSeriesResponse, ValidateSpatialRequest,
        ValidateSpatialResponse,
    },
};
use dagmar::{Dag, NodeId};
use futures::{stream::FuturesUnordered, Stream};
use std::{collections::HashMap, net::SocketAddr, pin::Pin};
use thiserror::Error;
use tokio::sync::mpsc::{channel, Receiver};
use tokio_stream::{
    wrappers::{ReceiverStream, UnixListenerStream},
    StreamExt,
};
use tonic::{transport::Server, Request, Response, Status};

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("test name {0} not found in dag")]
    TestNotInDag(String),
    // #[error("failed to run test")]
    // Runner(#[from] runner::Error),
}

type SeriesResponseStream =
    Pin<Box<dyn Stream<Item = Result<ValidateSeriesResponse, Status>> + Send>>;
type SpatialResponseStream =
    Pin<Box<dyn Stream<Item = Result<ValidateSpatialResponse, Status>> + Send>>;

pub enum ListenerType {
    Addr(SocketAddr),
    UnixListener(UnixListenerStream),
}

#[derive(Debug)]
pub struct RoveService<'a> {
    // TODO: the String here can probably be &'a or &'static str
    // TODO: separate DAGs for series and spatial tests?
    dag: Dag<String>,
    data_switch: DataSwitch<'a>,
}

impl<'a> RoveService<'a> {
    pub fn new(dag: Dag<String>, data_switch: DataSwitch<'a>) -> Self {
        RoveService { dag, data_switch }
    }

    fn construct_subdag(&self, required_nodes: Vec<String>) -> Result<Dag<String>, Error> {
        fn add_descendants(
            dag: &Dag<String>,
            subdag: &mut Dag<String>,
            curr_index: NodeId,
            nodes_visited: &mut HashMap<NodeId, NodeId>,
        ) {
            for child_index in dag.nodes.get(curr_index).unwrap().children.iter() {
                if let Some(new_index) = nodes_visited.get(child_index) {
                    subdag.add_edge(*nodes_visited.get(&curr_index).unwrap(), *new_index);
                } else {
                    let new_index =
                        subdag.add_node(dag.nodes.get(*child_index).unwrap().elem.clone());
                    subdag.add_edge(*nodes_visited.get(&curr_index).unwrap(), new_index);

                    nodes_visited.insert(*child_index, new_index);

                    add_descendants(dag, subdag, *child_index, nodes_visited);
                }
            }
        }

        let mut subdag = Dag::new();

        // this maps NodeIds from the dag to NodeIds from the subdag
        let mut nodes_visited: HashMap<NodeId, NodeId> = HashMap::new();

        for required in required_nodes.into_iter() {
            let index = self
                .dag
                .index_lookup
                .get(&required)
                .ok_or(Error::TestNotInDag(required))?;

            if !nodes_visited.contains_key(index) {
                let subdag_index =
                    subdag.add_node(self.dag.nodes.get(*index).unwrap().elem.clone());

                nodes_visited.insert(*index, subdag_index);

                add_descendants(&self.dag, &mut subdag, *index, &mut nodes_visited);
            }
        }

        Ok(subdag)
    }

    fn schedule_tests_series(
        subdag: Dag<String>,
        data: SeriesCache,
    ) -> Receiver<Result<ValidateSeriesResponse, Status>> {
        // spawn and channel are required if you want handle "disconnect" functionality
        // the `out_stream` will not be polled after client disconnect
        let (tx, rx) = channel(subdag.nodes.len());
        tokio::spawn(async move {
            let mut children_completed_map: HashMap<NodeId, usize> = HashMap::new();
            let mut test_futures = FuturesUnordered::new();

            for leaf_index in subdag.leaves.clone().into_iter() {
                test_futures.push(harness::run_test_series(
                    subdag.nodes.get(leaf_index).unwrap().elem.as_str(),
                    &data,
                ));
            }

            while let Some(res) = test_futures.next().await {
                match tx
                    .send(
                        res.clone()
                            .map_err(|e| Status::aborted(format!("a test run failed: {}", e))),
                    )
                    .await
                {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }

                match res {
                    Ok(inner) => {
                        let completed_index = subdag.index_lookup.get(inner.test.as_str()).unwrap();

                        for parent_index in
                            subdag.nodes.get(*completed_index).unwrap().parents.iter()
                        {
                            let children_completed = children_completed_map
                                .get(parent_index)
                                .map(|x| x + 1)
                                .unwrap_or(1);

                            children_completed_map.insert(*parent_index, children_completed);

                            if children_completed
                                >= subdag.nodes.get(*parent_index).unwrap().children.len()
                            {
                                test_futures.push(harness::run_test_series(
                                    subdag.nodes.get(*parent_index).unwrap().elem.as_str(),
                                    &data,
                                ))
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        rx
    }

    // sad about the amount of repetition here... perhaps we can do better once async
    // closures drop?
    fn schedule_tests_spatial(
        subdag: Dag<String>,
        data: SpatialCache,
    ) -> Receiver<Result<ValidateSpatialResponse, Status>> {
        // spawn and channel are required if you want handle "disconnect" functionality
        // the `out_stream` will not be polled after client disconnect
        let (tx, rx) = channel(subdag.nodes.len());
        tokio::spawn(async move {
            let mut children_completed_map: HashMap<NodeId, usize> = HashMap::new();
            let mut test_futures = FuturesUnordered::new();

            for leaf_index in subdag.leaves.clone().into_iter() {
                test_futures.push(harness::run_test_spatial(
                    subdag.nodes.get(leaf_index).unwrap().elem.as_str(),
                    &data,
                ));
            }

            while let Some(res) = test_futures.next().await {
                match tx
                    .send(
                        res.clone()
                            .map_err(|e| Status::aborted(format!("a test run failed: {}", e))),
                    )
                    .await
                {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }

                match res {
                    Ok(inner) => {
                        let completed_index = subdag.index_lookup.get(inner.test.as_str()).unwrap();

                        for parent_index in
                            subdag.nodes.get(*completed_index).unwrap().parents.iter()
                        {
                            let children_completed = children_completed_map
                                .get(parent_index)
                                .map(|x| x + 1)
                                .unwrap_or(1);

                            children_completed_map.insert(*parent_index, children_completed);

                            if children_completed
                                >= subdag.nodes.get(*parent_index).unwrap().children.len()
                            {
                                test_futures.push(harness::run_test_spatial(
                                    subdag.nodes.get(*parent_index).unwrap().elem.as_str(),
                                    &data,
                                ))
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        rx
    }

    // TODO: rethink string and error types here
    pub async fn validate_series_direct(
        &self,
        series_id: String,
        tests: Vec<String>,
        timerange: Timerange,
    ) -> Result<Receiver<Result<ValidateSeriesResponse, Status>>, Status> {
        if tests.is_empty() {
            return Err(Status::invalid_argument(
                "request must specify at least 1 test to be run",
            ));
        }

        let data = match self
            .data_switch
            .get_series_data(series_id.as_str(), timerange, 2)
            .await
        {
            Ok(data) => data,
            Err(e) => {
                tracing::error!(%e);
                return Err(Status::not_found(format!(
                    "data not found by data_switch: {}",
                    e
                )));
            }
        };

        let subdag = self
            .construct_subdag(tests)
            .map_err(|e| Status::not_found(format!("failed to construct subdag: {}", e)))?;

        Ok(RoveService::schedule_tests_series(subdag, data))
    }

    pub async fn validate_spatial_direct(
        &self,
        spatial_id: String,
        tests: Vec<String>,
        polygon: Vec<GeoPoint>,
        time: Timestamp,
    ) -> Result<Receiver<Result<ValidateSpatialResponse, Status>>, Status> {
        if tests.is_empty() {
            return Err(Status::invalid_argument(
                "request must specify at least 1 test to be run",
            ));
        }

        let data = match self
            .data_switch
            .get_spatial_data(polygon, spatial_id.as_str(), time)
            .await
        {
            Ok(data) => data,
            Err(e) => {
                tracing::error!(%e);
                return Err(Status::not_found(format!(
                    "data not found by data_switch: {}",
                    e
                )));
            }
        };

        let subdag = self
            .construct_subdag(tests)
            .map_err(|e| Status::not_found(format!("failed to construct subdag: {}", e)))?;

        Ok(RoveService::schedule_tests_spatial(subdag, data))
    }
}

#[tonic::async_trait]
impl Rove for RoveService<'static> {
    type ValidateSeriesStream = SeriesResponseStream;
    type ValidateSpatialStream = SpatialResponseStream;

    #[tracing::instrument]
    async fn validate_series(
        &self,
        request: Request<ValidateSeriesRequest>,
    ) -> Result<Response<Self::ValidateSeriesStream>, Status> {
        tracing::debug!("Got a request: {:?}", request);

        let req = request.into_inner();

        let rx = self
            .validate_series_direct(
                req.series_id,
                req.tests,
                Timerange {
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
            )
            .await?;

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::ValidateSeriesStream
        ))
    }

    #[tracing::instrument]
    async fn validate_spatial(
        &self,
        request: Request<ValidateSpatialRequest>,
    ) -> Result<Response<Self::ValidateSpatialStream>, Status> {
        tracing::debug!("Got a request: {:?}", request);

        let req = request.into_inner();

        let rx = self
            .validate_spatial_direct(
                req.spatial_id,
                req.tests,
                req.polygon,
                Timestamp(
                    req.time
                        .as_ref()
                        .ok_or(Status::invalid_argument("invalid timestamp for start_time"))?
                        .seconds,
                ),
            )
            .await?;

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::ValidateSpatialStream
        ))
    }
}

pub async fn start_server(
    listener: ListenerType,
    data_switch: DataSwitch<'static>,
    dag: Dag<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    match listener {
        ListenerType::Addr(addr) => {
            let rove_service = RoveService::new(dag, data_switch);

            tracing::info!(message = "Starting server.", %addr);

            Server::builder()
                .trace_fn(|_| tracing::info_span!("helloworld_server"))
                .add_service(RoveServer::new(rove_service))
                .serve(addr)
                .await?;
        }
        ListenerType::UnixListener(stream) => {
            let rove_service = RoveService::new(dag, data_switch);

            Server::builder()
                .add_service(RoveServer::new(rove_service))
                .serve_with_incoming(stream)
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn construct_fake_dag() -> Dag<String> {
        let mut dag: Dag<String> = Dag::new();

        let test6 = dag.add_node(String::from("test6"));

        let test4 = dag.add_node_with_children(String::from("test4"), vec![test6]);
        let test5 = dag.add_node_with_children(String::from("test5"), vec![test6]);

        let test2 = dag.add_node_with_children(String::from("test2"), vec![test4]);
        let test3 = dag.add_node_with_children(String::from("test3"), vec![test5]);

        let _test1 = dag.add_node_with_children(String::from("test1"), vec![test2, test3]);

        dag
    }

    #[test]
    fn test_construct_subdag() {
        let rove_service = RoveService::new(construct_fake_dag(), DataSwitch::new(HashMap::new()));

        assert_eq!(rove_service.dag.count_edges(), 6);

        let subdag = rove_service
            .construct_subdag(vec![String::from("test4")])
            .unwrap();

        assert_eq!(subdag.count_edges(), 1);

        let subdag = rove_service
            .construct_subdag(vec![String::from("test1")])
            .unwrap();

        assert_eq!(subdag.count_edges(), 6);
    }
}
