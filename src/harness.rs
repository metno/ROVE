use crate::{
    data_switch::DataCache,
    pb::{Flag, TestResult, ValidateResponse},
    pipeline::{CheckConf, PipelineStep},
};
use chrono::prelude::*;
use chronoutil::DateRule;
use olympian::{
    checks::{
        series::{spike_check_cache, step_check_cache},
        spatial::{buddy_check_cache, sct_cache, BuddyCheckArgs, SctArgs},
    },
    SingleOrVec, Timeseries,
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

pub fn run_test(step: &PipelineStep, cache: &DataCache) -> Result<ValidateResponse, Error> {
    let step_name = step.name.to_string();

    let flags: Vec<Timeseries<olympian::Flag>> = match &step.check {
        CheckConf::SpikeCheck(conf) => spike_check_cache(cache, conf.max)?,
        CheckConf::StepCheck(conf) => step_check_cache(cache, conf.max)?,
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
        _ => {
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

    let date_rule = DateRule::new(
        // TODO: make sure this start time is actually correct
        Utc.timestamp_opt(cache.start_time.0, 0).unwrap(),
        cache.period,
    );
    let results = flags
        .into_iter()
        .flat_map(|flag_series| {
            flag_series
                .values
                .into_iter()
                .zip(date_rule)
                .zip(std::iter::repeat(flag_series.tag))
        })
        .map(|((flag, time), identifier)| {
            let flag: Flag = flag.try_into().map_err(Error::UnknownFlag)?;
            Ok(TestResult {
                time: Some(prost_types::Timestamp {
                    seconds: time.timestamp(),
                    nanos: 0,
                }),
                identifier,
                flag: flag.into(),
            })
        })
        .collect::<Result<Vec<TestResult>, Error>>()?;

    Ok(ValidateResponse {
        test: step_name,
        results,
    })
}
