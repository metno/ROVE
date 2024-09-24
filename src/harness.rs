use crate::{
    data_switch::DataCache,
    pb::{Flag, TestResult, ValidateResponse},
};
use chrono::prelude::*;
use chronoutil::DateRule;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum Error {
    #[error("test name {0} not found in runner")]
    InvalidTestName(String),
    #[error("failed to run test")]
    FailedTest(#[from] olympian::Error),
    #[error("unknown olympian flag: {0}")]
    UnknownFlag(String),
}

// TODO: make sure we aren't feeding leading/trailing values to the spatial tests
pub async fn run_test(test: &str, cache: &DataCache) -> Result<ValidateResponse, Error> {
    let flags: Vec<(String, Vec<Flag>)> = match test {
        // TODO: put these in a lookup table?
        "dip_check" => {
            const LEADING_PER_RUN: u8 = 2;

            // TODO: use actual test params
            // TODO: use par_iter?

            let mut result_vec = Vec::with_capacity(cache.data.len());

            // NOTE: Does data in each series have the same len?
            let series_len = cache.data[0].1.len();

            for i in 0..cache.data.len() {
                result_vec.push((
                    cache.data[i].0.clone(),
                    cache.data[i].1
                        [(cache.num_leading_points - LEADING_PER_RUN).into()..series_len]
                        .windows((LEADING_PER_RUN + 1).into())
                        .map(|window| {
                            olympian::dip_check(window, 2., 3.)?
                                .try_into()
                                .map_err(Error::UnknownFlag)
                        })
                        .collect::<Result<Vec<Flag>, Error>>()?,
                ))
            }
            result_vec
        }
        "step_check" => {
            const LEADING_PER_RUN: u8 = 1;

            let mut result_vec = Vec::with_capacity(cache.data.len());

            // NOTE: Does data in each series have the same len?
            let series_len = cache.data[0].1.len();

            for i in 0..cache.data.len() {
                result_vec.push((
                    cache.data[i].0.clone(),
                    cache.data[i].1
                        [(cache.num_leading_points - LEADING_PER_RUN).into()..series_len]
                        .windows((LEADING_PER_RUN + 1).into())
                        .map(|window| {
                            olympian::step_check(window, 2., 3.)?
                                .try_into()
                                .map_err(Error::UnknownFlag)
                        })
                        .collect::<Result<Vec<Flag>, Error>>()?,
                ))
            }
            result_vec
        }
        "buddy_check" => {
            let n = cache.data.len();

            let series_len = cache.data[0].1.len();

            let mut result_vec: Vec<(String, Vec<Flag>)> = cache
                .data
                .iter()
                .map(|ts| (ts.0.clone(), Vec::with_capacity(series_len)))
                .collect();

            for i in 0..series_len {
                // TODO: change `buddy_check` to accept Option<f32>?
                let inner: Vec<f32> = cache.data.iter().map(|v| v.1[i].unwrap()).collect();

                let spatial_result = olympian::buddy_check(
                    &cache.rtree,
                    &inner,
                    &vec![5000.; n],
                    &vec![2; n],
                    2.,
                    200.,
                    0.,
                    1.,
                    2,
                    &vec![true; n],
                )?;

                if spatial_result
                    .iter()
                    .any(|flag| Flag::try_from(*flag).is_err())
                {
                    return Err(Error::UnknownFlag(
                        // this is messy, but at least it's not on the critical path
                        // and it lets the critical path code be more efficient
                        Flag::try_from(
                            *spatial_result
                                .iter()
                                .find(|flag| Flag::try_from(**flag).is_err())
                                .unwrap(),
                        )
                        .err()
                        .unwrap(),
                    ));
                }

                for (i, flag) in spatial_result
                    .into_iter()
                    .map(|flag| flag.try_into().unwrap())
                    .enumerate()
                {
                    result_vec[i].1.push(flag);
                }
            }
            result_vec
        }
        "sct" => {
            let n = cache.data.len();

            let series_len = cache.data[0].1.len();

            let mut result_vec: Vec<(String, Vec<Flag>)> = cache
                .data
                .iter()
                .map(|ts| (ts.0.clone(), Vec::with_capacity(series_len)))
                .collect();

            for i in 0..series_len {
                // TODO: change `sct` to accept Option<f32>?
                let inner: Vec<f32> = cache.data.iter().map(|v| v.1[i].unwrap()).collect();
                let spatial_result = olympian::sct(
                    &cache.rtree,
                    &inner,
                    5,
                    100,
                    50000.,
                    150000.,
                    5,
                    20,
                    200.,
                    10000.,
                    200.,
                    &vec![4.; n],
                    &vec![8.; n],
                    &vec![0.5; n],
                    None,
                )?;

                if spatial_result
                    .iter()
                    .any(|flag| Flag::try_from(*flag).is_err())
                {
                    return Err(Error::UnknownFlag(
                        // this is messy, but at least it's not on the critical path
                        // and it lets the critical path code be more efficient
                        Flag::try_from(
                            *spatial_result
                                .iter()
                                .find(|flag| Flag::try_from(**flag).is_err())
                                .unwrap(),
                        )
                        .err()
                        .unwrap(),
                    ));
                }

                for (i, flag) in spatial_result
                    .into_iter()
                    .map(|flag| flag.try_into().unwrap())
                    .enumerate()
                {
                    result_vec[i].1.push(flag);
                }
            }
            result_vec
        }
        _ => {
            // used for integration testing
            if test.starts_with("test") {
                vec![("test".to_string(), vec![Flag::Inconclusive])]
            } else {
                return Err(Error::InvalidTestName(test.to_string()));
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
                .1
                .into_iter()
                .zip(date_rule)
                .zip(std::iter::repeat(flag_series.0))
        })
        .map(|((flag, time), identifier)| TestResult {
            time: Some(prost_types::Timestamp {
                seconds: time.timestamp(),
                nanos: 0,
            }),
            identifier,
            flag: flag.into(),
        })
        .collect();

    Ok(ValidateResponse {
        test: test.to_string(),
        results,
    })
}
