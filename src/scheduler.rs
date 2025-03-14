//! Utilities for scheduling QC checks

use crate::{
    data_switch::{self, DataCache, DataSwitch, SpaceSpec, TimeSpec},
    harness::{self, CheckResult},
    pipeline::Pipeline,
};
use std::collections::HashMap;
use thiserror::Error;

/// Error type for Scheduler methods
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// The check harness returned an error
    #[error("failed to run check: {0}")]
    Runner(#[from] harness::Error),
    /// The method received an invalid argument
    #[error("invalid argument: {0}")]
    InvalidArg(&'static str),
    /// The [`DataSwitch`] returned an error
    #[error("data switch failed to find data: {0}")]
    DataSwitch(#[from] data_switch::Error),
}

/// Receiver type for QC runs
///
/// Holds information about test pipelines and data sources
#[derive(Debug)]
pub struct Scheduler {
    // this is pub so that the server can determine the number of checks in a pipeline to size
    // its channel with. can be made private if the server functionality is deprecated
    #[allow(missing_docs)]
    pub pipelines: HashMap<String, Pipeline>,
    data_switch: DataSwitch,
}

impl Scheduler {
    /// Instantiate a new scheduler
    pub fn new(pipelines: HashMap<String, Pipeline>, data_switch: DataSwitch) -> Self {
        Scheduler {
            pipelines,
            data_switch,
        }
    }

    /// Directly invoke a Pipeline on a Datacache. If you want the scheduler to fetch the Pipeline
    /// and DataCache for you, see [`validate_direct`](Scheduler::validate_direct).
    pub fn schedule_tests(pipeline: &Pipeline, data: DataCache) -> Result<Vec<CheckResult>, Error> {
        pipeline
            .steps
            .iter()
            .map(|step| harness::run_check(step, &data).map_err(Error::Runner))
            .collect()
    }

    /// Run a set of QC tests on some data
    ///
    /// `data_source` is the key identifying a connector in the
    /// [`DataSwitch`].
    /// `backing_sources` a list of keys similar to `data_source`, but data
    /// from these will only be used to QC data from `data_source` and will not
    /// themselves be QCed.
    /// `time_spec` and `space_spec` narrow down what data to QC, more info
    /// on what these mean and how to construct them can be found on their
    /// own doc pages.
    /// `test_pipeline` represents the pipeline of checks to be run. Available
    /// options of pipelines are defined at load time for the service, where
    /// pipelines are read from toml files.
    /// `extra_spec` is an extra identifier that gets passed to the relevant
    /// DataConnector. The format of `extra_spec` is connector-specific.
    ///
    /// # Errors
    ///
    /// Returned from the function if:
    /// - The pipeline named by in the `test_pipeline` argument is not recognized
    ///   by the system
    /// - The data_source string did not have a matching entry in the
    ///   Scheduler's DataSwitch
    ///
    /// In the the returned channel if:
    /// - The test harness encounters an error on during one of the QC tests.
    ///   This will also result in the channel being closed
    pub async fn validate_direct(
        &self,
        data_source: impl AsRef<str>,
        // TODO: we should actually use these
        _backing_sources: &[impl AsRef<str>],
        time_spec: &TimeSpec,
        space_spec: &SpaceSpec,
        // TODO: should we allow specifying multiple pipelines per call?
        test_pipeline: impl AsRef<str>,
        extra_spec: Option<&str>,
    ) -> Result<Vec<CheckResult>, Error> {
        let pipeline = self
            .pipelines
            .get(test_pipeline.as_ref())
            .ok_or(Error::InvalidArg("pipeline not recognised"))?;

        let data = match self
            .data_switch
            .fetch_data(
                data_source.as_ref(),
                space_spec,
                time_spec,
                pipeline.num_leading_required,
                pipeline.num_trailing_required,
                extra_spec,
            )
            .await
        {
            Ok(data) => data,
            Err(e) => {
                tracing::error!(%e);
                return Err(Error::DataSwitch(e));
            }
        };

        Scheduler::schedule_tests(pipeline, data)
    }
}
