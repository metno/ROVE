//! Definitions and utilities for deserialising QC pipelines

use olympian::checks::series::{
    SPIKE_LEADING_PER_RUN, SPIKE_TRAILING_PER_RUN, STEP_LEADING_PER_RUN,
};
use serde::Deserialize;
use std::{collections::HashMap, path::Path};
use thiserror::Error;

/// Data structure defining a pipeline of checks, with parameters built in
///
/// Rather than constructing these manually, a convenience function `load_pipelines` is provided
/// to deserialize a set of pipelines from a directory containing TOML files defining them. Users
/// may still want to write their own implementation of load_pipelines in case they want to encode
/// extra information in the pipeline definitions, or use a different directory structure
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct Pipeline {
    /// Sequence of steps in the pipeline
    #[serde(rename = "step")]
    pub steps: Vec<PipelineStep>,
    /// Number of leading points required by the checks in this pipeline
    #[serde(skip)]
    pub num_leading_required: u8,
    /// Number of trailing points required by the checks in this pipeline
    #[serde(skip)]
    pub num_trailing_required: u8,
}

/// One step in a pipeline
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct PipelineStep {
    /// Name of the step
    ///
    /// This is kept distinct from the name of the check, as one check may be used for several
    /// different steps with different purposes within a pipeline. Most often this will simply
    /// shadow the name of the check though.
    pub name: String,
    /// Defines which check is to be used for this step, along with a configuration for that check
    #[serde(flatten)]
    pub check: CheckConf,
}

/// Identifies a check, and provides a configuration (arguments) for it
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum CheckConf {
    SpecialValueCheck(SpecialValueCheckConf),
    RangeCheck(RangeCheckConf),
    RangeCheckDynamic(RangeCheckDynamicConf),
    StepCheck(StepCheckConf),
    SpikeCheck(SpikeCheckConf),
    FlatlineCheck(FlatlineCheckConf),
    BuddyCheck(BuddyCheckConf),
    Sct(SctConf),
    ModelConsistencyCheck(ModelConsistencyCheckConf),
    /// Mock pipeline used for testing
    #[serde(skip)]
    Dummy,
}

impl CheckConf {
    fn get_num_leading_trailing(&self) -> (u8, u8) {
        match self {
            CheckConf::SpecialValueCheck(_)
            | CheckConf::RangeCheck(_)
            | CheckConf::RangeCheckDynamic(_)
            | CheckConf::BuddyCheck(_)
            | CheckConf::Sct(_)
            | CheckConf::ModelConsistencyCheck(_)
            | CheckConf::Dummy => (0, 0),
            CheckConf::StepCheck(_) => (STEP_LEADING_PER_RUN, 0),
            CheckConf::SpikeCheck(_) => (SPIKE_LEADING_PER_RUN, SPIKE_TRAILING_PER_RUN),
            CheckConf::FlatlineCheck(conf) => (conf.max, 0),
        }
    }
}

/// See [`olympian::checks::single::special_values_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct SpecialValueCheckConf {
    pub special_values: Vec<f64>,
}

/// See [`olympian::checks::single::range_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct RangeCheckConf {
    pub max: f64,
    pub min: f64,
}

// TODO: document this once we have a concrete impl to base docs on
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct RangeCheckDynamicConf {
    pub source: String,
}

/// See [`olympian::checks::series::step_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct StepCheckConf {
    pub max: f64,
}

/// See [`olympian::checks::series::spike_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct SpikeCheckConf {
    pub max: f64,
}

/// See [`olympian::checks::series::flatline_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct FlatlineCheckConf {
    pub max: u8,
}

/// See [`olympian::checks::spatial::buddy_check`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct BuddyCheckConf {
    pub radii: f64,
    pub min_buddies: u32,
    pub threshold: f64,
    pub max_elev_diff: f64,
    pub elev_gradient: f64,
    pub min_std: f64,
    pub num_iterations: u32,
}

/// See [`olympian::checks::spatial::sct`]
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct SctConf {
    pub num_min: usize,
    pub num_max: usize,
    pub inner_radius: f64,
    pub outer_radius: f64,
    pub num_iterations: u32,
    pub num_min_prof: usize,
    pub min_elev_diff: f64,
    pub min_horizontal_scale: f64,
    pub vertical_scale: f64,
    pub pos: f64,
    pub neg: f64,
    pub eps2: f64,
    pub obs_to_check: Option<Vec<bool>>,
}

// TODO: document this once we have a concrete impl to base docs on
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[allow(missing_docs)]
pub struct ModelConsistencyCheckConf {
    pub model_source: String,
    pub model_args: String,
    pub threshold: f64,
}

/// Errors relating to pipeline deserialization
#[derive(Error, Debug)]
pub enum Error {
    /// Generic IO error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// TOML deserialize error
    #[error("failed to deserialize toml: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    /// The directory contained something that wasn't a file
    #[error("the directory contained something that wasn't a file")]
    DirectoryStructure,
    /// Pipeline filename could not be parsed as a unicode string
    #[error("pipeline filename could not be parsed as a unicode string")]
    InvalidFilename,
}

/// Given a pipeline, derive the number of leading and trailing points per timeseries needed in
/// a dataset, for all the intended data to be QCed by the pipeline
pub fn derive_num_leading_trailing(pipeline: &Pipeline) -> (u8, u8) {
    pipeline
        .steps
        .iter()
        .map(|step| step.check.get_num_leading_trailing())
        .fold((0, 0), |acc, x| (acc.0.max(x.0), acc.1.max(x.1)))
}

/// Given a directory containing toml files that each define a check pipeline, construct a hashmap
/// of pipelines, where the keys are the pipelines' names (filename of the toml file that defines
/// them, without the file extension)
pub fn load_pipelines(path: impl AsRef<Path>) -> Result<HashMap<String, Pipeline>, Error> {
    std::fs::read_dir(path)?
        // transform dir entries into (String, Pipeline) pairs
        .map(|entry| {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(Error::DirectoryStructure);
            }

            let name = entry
                .file_name()
                .to_str()
                .ok_or(Error::InvalidFilename)?
                .trim_end_matches(".toml")
                .to_string();

            let mut pipeline = toml::from_str(&std::fs::read_to_string(entry.path())?)?;
            (
                pipeline.num_leading_required,
                pipeline.num_trailing_required,
            ) = derive_num_leading_trailing(&pipeline);

            Ok(Some((name, pipeline)))
        })
        // remove `None`s
        .filter_map(Result::transpose)
        // collect to hash map
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_deserialize_fresh() {
        load_pipelines("sample_pipelines/fresh")
            .unwrap()
            .get("TA_PT1H")
            .unwrap();
    }
}
