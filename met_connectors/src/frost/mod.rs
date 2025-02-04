use async_trait::async_trait;
use chrono::prelude::*;
use rove::{
    data_switch,
    data_switch::{DataCache, DataConnector, SpaceSpec, TimeSpec},
};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

mod duration;
mod fetch;
mod util;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("{0}")]
    InvalidElementId(&'static str),
    #[error("invalid space_spec: {0}")]
    InvalidSpaceSpec(&'static str),
    #[error("fetching data from frost failed")]
    Request(#[from] reqwest::Error),
    #[error("failed to find obs in json body: {0}")]
    FindObs(String),
    #[error("failed to find location in json body: {0}")]
    FindLocation(String),
    #[error("failed to deserialise data to struct")]
    DeserializeObs(#[from] serde_json::Error),
    #[error("failed to find metadata in json body: {0}")]
    FindMetadata(String),
    #[error("duration parser failed, invalid duration: {input}")]
    ParseDuration {
        source: duration::Error,
        input: String,
    },
    #[error("{0}")]
    MissingObs(String),
    #[error("{0}")]
    Misalignment(String),
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug)]
pub struct Frost {
    pub credentials: Credentials,
}

#[derive(Deserialize, Debug)]
struct FrostObsBody {
    #[serde(deserialize_with = "des_value")]
    value: f64,
}

// TODO: flatten this with FrostObsBody?
#[derive(Deserialize, Debug)]
struct FrostObs {
    body: FrostObsBody,
    #[serde(deserialize_with = "des_time")]
    time: DateTime<Utc>,
}

#[derive(Deserialize, Debug)]
struct FrostLatLonElev {
    #[serde(rename = "elevation(masl/hs)")]
    #[serde(deserialize_with = "des_value")]
    elevation: f64,
    #[serde(deserialize_with = "des_value")]
    latitude: f64,
    #[serde(deserialize_with = "des_value")]
    longitude: f64,
}

#[derive(Deserialize, Debug)]
struct FrostLocation {
    #[serde(deserialize_with = "des_time")]
    from: DateTime<Utc>,
    #[serde(deserialize_with = "des_time")]
    to: DateTime<Utc>,
    value: FrostLatLonElev,
}

fn des_value<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
    D::Error: serde::de::Error,
{
    use serde::de::Error;
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse().map_err(D::Error::custom)
}

fn des_time<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
    D::Error: serde::de::Error,
{
    use serde::de::Error;
    let s: String = Deserialize::deserialize(deserializer)?;
    Ok(chrono::DateTime::parse_from_rfc3339(s.as_str())
        .map_err(D::Error::custom)?
        .with_timezone(&Utc))
}

#[async_trait]
impl DataConnector for Frost {
    async fn fetch_data(
        &self,
        space_spec: &SpaceSpec,
        time_spec: &TimeSpec,
        num_leading_points: u8,
        num_trailing_points: u8,
        extra_spec: Option<&str>,
    ) -> Result<DataCache, data_switch::Error> {
        fetch::fetch_data_inner(
            space_spec,
            time_spec,
            num_leading_points,
            num_trailing_points,
            extra_spec,
            &self.credentials,
        )
        .await
    }
}
