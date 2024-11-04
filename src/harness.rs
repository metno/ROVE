use crate::{
    data_switch::DataCache,
    pipeline::{CheckConf, PipelineStep},
};
use olympian::{
    checks::{
        series::{flatline_check_cache, spike_check_cache, step_check_cache},
        single::{range_check_cache, special_values_check_cache},
        spatial::{buddy_check_cache, sct_cache, BuddyCheckArgs, SctArgs},
    },
    Flag, SingleOrVec, Timeseries,
};
use thiserror::Error;

#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum Error {
    #[error("test name {0} not found in runner")]
    InvalidTestName(String),
    #[error("failed to run test: {0}")]
    FailedTest(#[from] olympian::Error),
    #[error("unknown olympian flag: {0}")]
    UnknownFlag(String),
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub check: String,
    pub results: Vec<Timeseries<Flag>>,
}

pub fn run_check(step: &PipelineStep, cache: &DataCache) -> Result<CheckResult, Error> {
    let step_name = step.name.to_string();

    let flags: Vec<Timeseries<Flag>> = match &step.check {
        CheckConf::SpecialValueCheck(conf) => {
            special_values_check_cache(cache, &conf.special_values)
        }
        CheckConf::RangeCheck(conf) => range_check_cache(cache, conf.max, conf.min),
        CheckConf::RangeCheckDynamic(_conf) => todo!(), // TODO: need model data
        CheckConf::StepCheck(conf) => step_check_cache(cache, conf.max)?,
        CheckConf::SpikeCheck(conf) => spike_check_cache(cache, conf.max)?,
        // TODO: evaluate the threshold for flaline_check
        CheckConf::FlatlineCheck(conf) => flatline_check_cache(cache, conf.max, 0.0001)?,
        CheckConf::BuddyCheck(conf) => buddy_check_cache(
            cache,
            // TODO: from/into impl?
            &BuddyCheckArgs {
                radii: SingleOrVec::Single(conf.radii),
                min_buddies: SingleOrVec::Single(conf.min_buddies),
                threshold: conf.threshold,
                max_elev_diff: conf.max_elev_diff,
                elev_gradient: conf.elev_gradient,
                min_std: conf.min_std,
                num_iterations: conf.num_iterations,
            },
            None,
        )?,
        CheckConf::Sct(conf) => sct_cache(
            cache,
            &SctArgs {
                num_min: conf.num_min,
                num_max: conf.num_max,
                inner_radius: conf.inner_radius,
                outer_radius: conf.outer_radius,
                num_iterations: conf.num_iterations,
                num_min_prof: conf.num_min_prof,
                min_elev_diff: conf.min_elev_diff,
                min_horizontal_scale: conf.min_horizontal_scale,
                vertical_scale: conf.vertical_scale,
                pos: SingleOrVec::Single(conf.pos),
                neg: SingleOrVec::Single(conf.neg),
                eps2: SingleOrVec::Single(conf.eps2),
            },
            None,
        )?,
        CheckConf::ModelConsistencyCheck(_conf) => todo!(), // TODO: need model data
        CheckConf::Dummy => {
            // used for integration testing
            if step_name.starts_with("test") {
                vec![Timeseries {
                    tag: "test".to_string(),
                    values: vec![olympian::Flag::Inconclusive],
                }]
            } else {
                return Err(Error::InvalidTestName(step_name));
            }
        }
    };

    Ok(CheckResult {
        check: step_name,
        results: flags,
    })
}
