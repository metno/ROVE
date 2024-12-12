//! System for quality control of meteorological data.
//!
//! Provides a modular system for scheduling QC tests on data, and marshalling
//! data and metadata into these tests. It can be used as a standalone gRPC
//! service, or a component within another service (say, a data ingestor).
//!
//! As a standalone service:
//! ```no_run
//! use rove::{
//!     start_server,
//!     data_switch::{DataSwitch, DataConnector},
//!     dev_utils::{TestDataSource, construct_hardcoded_pipeline},
//! };
//! use std::collections::HashMap;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let data_switch = DataSwitch::new(HashMap::from([(
//!         String::from("test"),
//!         Box::new(TestDataSource{
//!             data_len_single: 3,
//!             data_len_series: 1000,
//!             data_len_spatial: 1000,
//!         }) as Box<dyn DataConnector + Send>
//!     )]));
//!
//!     start_server(
//!         "[::1]:1337".parse()?,
//!         data_switch,
//!         construct_hardcoded_pipeline(),
//!     )
//!     .await
//! }
//! ```
//!
//! As a component:
//! ```no_run
//! use rove::{
//!     Scheduler,
//!     data_switch::{DataSwitch, DataConnector, Timestamp, Timerange, TimeSpec, SpaceSpec},
//!     dev_utils::{TestDataSource, construct_hardcoded_pipeline},
//! };
//! use std::collections::HashMap;
//! use chrono::{Utc, TimeZone};
//! use chronoutil::RelativeDuration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let data_switch = DataSwitch::new(HashMap::from([(
//!         String::from("test"),
//!         Box::new(TestDataSource{
//!             data_len_single: 3,
//!             data_len_series: 1000,
//!             data_len_spatial: 1000,
//!         }) as Box<dyn DataConnector + Send>
//!     )]));
//!
//!     let rove_scheduler = Scheduler::new(construct_hardcoded_pipeline(), data_switch);
//!
//!     let response = rove_scheduler.validate_direct(
//!         "my_data_source",
//!         &vec!["my_backing_source"],
//!         &TimeSpec::new(
//!             Timestamp(
//!                 Utc.with_ymd_and_hms(2023, 6, 26, 12, 0, 0)
//!                     .unwrap()
//!                     .timestamp(),
//!             ),
//!             Timestamp(
//!                 Utc.with_ymd_and_hms(2023, 6, 26, 14, 0, 0)
//!                     .unwrap()
//!                     .timestamp(),
//!             ),
//!             RelativeDuration::minutes(5),
//!         ),
//!         &SpaceSpec::One(String::from("station_id")),
//!         "TA_PT1H",
//!         None,
//!     ).await?;
//!
//!     for result in response {
//!         println!("check: {}", result.check);
//!         println!("flags: {:?}", result.results);
//!     }
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]

pub mod data_switch;
mod harness;
pub(crate) mod pb;
mod pipeline;
pub mod scheduler;
mod server;

pub use pipeline::{load_pipelines, Pipeline};

pub use scheduler::Scheduler;

pub use server::start_server;

#[doc(hidden)]
pub use server::start_server_unix_listener;

#[doc(hidden)]
pub mod dev_utils {
    use crate::{
        data_switch::{self, DataCache, DataConnector, SpaceSpec, TimeSpec, Timeseries, Timestamp},
        pipeline::{derive_num_leading_trailing, Pipeline},
    };
    use async_trait::async_trait;
    use chronoutil::RelativeDuration;
    use std::{collections::HashMap, hint::black_box};

    #[derive(Debug)]
    pub struct TestDataSource {
        pub data_len_single: usize,
        pub data_len_series: usize,
        pub data_len_spatial: usize,
    }

    #[async_trait]
    impl DataConnector for TestDataSource {
        async fn fetch_data(
            &self,
            space_spec: &SpaceSpec,
            _time_spec: &TimeSpec,
            num_leading_points: u8,
            num_trailing_points: u8,
            _extra_spec: Option<&str>,
        ) -> Result<DataCache, data_switch::Error> {
            match space_spec {
                SpaceSpec::One(data_id) => match data_id.as_str() {
                    // TODO: should we maybe be using time_spec for these instead of data_id?
                    // maybe something to come back to when we finalize the format of time_spec
                    "single" => black_box(Ok(DataCache::new(
                        vec![
                            Timeseries {
                                tag: "test".to_string(),
                                values: vec![Some(1.); self.data_len_single]
                            };
                            1
                        ],
                        vec![0.; 1],
                        vec![0.; 1],
                        vec![0.; 1],
                        Timestamp(0),
                        RelativeDuration::minutes(5),
                        num_leading_points,
                        num_trailing_points,
                    ))),
                    "series" => black_box(Ok(DataCache::new(
                        vec![
                            Timeseries {
                                tag: "test".to_string(),
                                values: vec![Some(1.); self.data_len_series]
                            };
                            1
                        ],
                        vec![0.; 1],
                        vec![0.; 1],
                        vec![0.; 1],
                        Timestamp(0),
                        RelativeDuration::minutes(5),
                        num_leading_points,
                        num_trailing_points,
                    ))),
                    _ => panic!("unknown data_id"),
                },
                SpaceSpec::All => black_box(Ok(DataCache::new(
                    vec![
                        Timeseries {
                            tag: "test".to_string(),
                            values: vec![
                                Some(1.);
                                num_leading_points as usize
                                    + 1
                                    + num_trailing_points as usize
                            ]
                        };
                        self.data_len_spatial
                    ],
                    (0..self.data_len_spatial)
                        .map(|i| ((i as f32).powi(2) * 0.001) % 3.)
                        .collect(),
                    (0..self.data_len_spatial)
                        .map(|i| ((i as f32 + 1.).powi(2) * 0.001) % 3.)
                        .collect(),
                    vec![1.; self.data_len_spatial],
                    Timestamp(0),
                    RelativeDuration::minutes(5),
                    num_leading_points,
                    num_trailing_points,
                ))),
                SpaceSpec::Polygon(_) => unimplemented!(),
            }
        }
    }

    // TODO: replace this by just loading a sample pipeline toml?
    pub fn construct_hardcoded_pipeline() -> HashMap<String, Pipeline> {
        let mut pipeline = toml::from_str(
            r#"
                    [[step]]
                    name = "step_check"
                    [step.step_check]
                    max = 3.0

                    [[step]]
                    name = "spike_check"
                    [step.spike_check]
                    max = 3.0

                    [[step]]
                    name = "buddy_check"
                    [step.buddy_check]
                    max = 3
                    radii = 5000.0
                    min_buddies = 2
                    threshold = 2.0
                    max_elev_diff = 200.0
                    elev_gradient = 0.0
                    min_std = 1.0
                    num_iterations =  2
                
                    [[step]]
                    name = "sct"
                    [step.sct]
                    num_min = 5
                    num_max = 100
                    inner_radius = 50000.0
                    outer_radius = 150000.0
                    num_iterations = 5
                    num_min_prof = 20
                    min_elev_diff = 200.0
                    min_horizontal_scale = 10000.0
                    vertical_scale = 200.0
                    pos = 4.0
                    neg = 8.0
                    eps2 = 0.5
            "#,
        )
        .unwrap();
        (
            pipeline.num_leading_required,
            pipeline.num_trailing_required,
        ) = derive_num_leading_trailing(&pipeline);

        HashMap::from([(String::from("hardcoded"), pipeline)])
    }
}
