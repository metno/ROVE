//! Utilities for creating and using [`DataConnector`](crate::data_switch::DataConnector)s
//!
//! Implementations of the [`DataConnector`](crate::data_switch::DataConnector)
//! trait are how ROVE accesses to data for QC. For any data source you wish ROVE to be able to pull data from, you must write an implementation of
//! [`DataConnector`](crate::data_switch::DataConnector) for it, and load that
//! connector into a [`DataSwitch`], which you then pass to
//! [`start_server`](crate::start_server) if using ROVE in gRPC
//! mode, or [`Scheduler::new`](crate::Scheduler::new)
//! otherwise.

pub use olympian::{DataCache, Timeseries, Timestamp};

use async_trait::async_trait;
use chronoutil::RelativeDuration;
use std::collections::HashMap;
use thiserror::Error;

/// Error type for DataSwitch
///
/// When implementing DataConnector, it may be helpful to implement your own
/// internal Error type, but it must ultimately be mapped to this type before
/// returning
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// The series_id was not in a valid format
    #[error("series id `{0}` could not be parsed")]
    InvalidSeriesId(String),
    /// The no connector was found for that data_source_id in the DataSwitch
    #[error("data source `{0}` not registered")]
    InvalidDataSource(String),
    /// The DataConnector or its data source could not parse the data_id
    #[error(
        "extra_spec `{extra_spec:?}` could not be parsed by data source {data_source}: {source}"
    )]
    InvalidExtraSpec {
        /// Name of the relevant data source
        data_source: &'static str,
        /// The data_id that could not be parsed
        extra_spec: Option<String>,
        /// The error in the DataConnector
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// Generic IO error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    // TODO: should we change these now we don't differentiate between series and spatial requests?
    // They are arguably still useful since fetching series vs spatial data is still a pattern some
    // sources may or may not support.
    /// The data source was asked for series data but does not offer it
    #[error("this data source does not offer series data: {0}")]
    UnimplementedSeries(String),
    /// The data source was asked for spatial data but does not offer it
    #[error("this data source does not offer spatial data: {0}")]
    UnimplementedSpatial(String),
    /// Failure to join a tokio task
    #[error("tokio task failure")]
    Join(#[from] tokio::task::JoinError),
    /// Catchall for any other errors that might occur inside a DataConnector object
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync + 'static>),
}

/// Inclusive range of time, from a start to end [`Timestamp`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timerange {
    /// Start of the timerange
    pub start: Timestamp,
    /// End of the timerange
    pub end: Timestamp,
}

/// Specifier of which data to fetch from a source by time, and time resolution
#[derive(Debug)]
pub struct TimeSpec {
    /// The range in time of data to fetch
    pub timerange: Timerange,
    /// The time resolution of data that should be fetched
    pub time_resolution: RelativeDuration,
}

impl TimeSpec {
    /// Construct a new `TimeSpec` with specified start and end timestamps, and
    /// a time resolution.
    pub fn new(start: Timestamp, end: Timestamp, time_resolution: RelativeDuration) -> Self {
        TimeSpec {
            timerange: Timerange { start, end },
            time_resolution,
        }
    }

    /// Alternative constructor for `TimeSpec` with time resolution specified
    /// using an ISO 8601 duration stamp, to avoid a dependency on chronoutil.
    pub fn new_time_resolution_string(
        start: Timestamp,
        end: Timestamp,
        time_resolution: &str,
    ) -> Result<Self, String> {
        Ok(TimeSpec {
            timerange: Timerange { start, end },
            time_resolution: RelativeDuration::parse_from_iso8601(time_resolution)
                .map_err(|e| e.to_string())?,
        })
    }
}

/// Specifier of geographic position, by latitude and longitude
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    /// latitude, in degrees
    pub lat: f32,
    /// longitude, in degrees
    pub lon: f32,
}

/// A geospatial polygon
///
/// represented by its vertices as a sequence of lat-lon points
pub type Polygon = Vec<GeoPoint>;

/// Specifier of which data to fetch from a source by location
#[derive(Debug)]
pub enum SpaceSpec {
    /// One single timeseries, specified with a data_id
    One(String),
    /// A Polygon in lat-lon space defining the area from which to fetch data
    Polygon(Polygon),
    /// The whole data set
    All,
}

/// Trait for pulling data from data sources
///
/// Uses [mod@async_trait]. It is recommended to tag your implementation with
/// the [`macro@async_trait`] macro to avoid having to deal with pinning,
/// futures, and lifetimes manually.
///
/// Here is an example implementation that just returns dummy data:
///
/// ```
/// use async_trait::async_trait;
/// use chronoutil::RelativeDuration;
/// use rove::data_switch::{self, *};
///
/// // You can use the receiver type to store anything that should persist
/// // between requests, i.e a connection pool
/// #[derive(Debug)]
/// struct TestDataSource;
///
/// #[async_trait]
/// impl DataConnector for TestDataSource {
///     async fn fetch_data(
///         &self,
///         // Specifier to restrict the data fetched spatially. Can be used
///         // to specify a single timeseries, all timeseries within a polygon,
///         // or the entire dataset.
///         _space_spec: &SpaceSpec,
///         // Specifier to restrict the data fetched by time.
///         time_spec: &TimeSpec,
///         // Some timeseries QC tests require extra data from before
///         // the start of the timerange to function. ROVE determines
///         // how many extra data points are needed, and passes that in
///         // here.
///         num_leading_points: u8,
///         // Similar to num_leading_points, but after the end of the
///         // timerange.
///         num_trailing_points: u8,
///         // Any extra string info your DataSource accepts, to further
///         // specify what data to fetch.
///         _extra_spec: Option<&str>,
///     ) -> Result<DataCache, data_switch::Error> {
///         // Here you can do whatever is need to fetch real data, whether
///         // that's a REST request, SQL call, NFS read etc.
///
///         Ok(DataCache::new(
///             vec![Timeseries{tag: String::from("identifier"), values: vec![Some(1.)]}],
///             vec![1.],
///             vec![1.],
///             vec![1.],
///             time_spec.timerange.start,
///             time_spec.time_resolution,
///             num_leading_points,
///             num_trailing_points,
///         ))
///     }
/// }
/// ```
///
/// Some real implementations can be found in [rove/met_connectors](https://github.com/metno/rove/tree/trunk/met_connectors)
#[async_trait]
pub trait DataConnector: Sync + std::fmt::Debug {
    /// fetch specified data from the data source
    async fn fetch_data(
        &self,
        space_spec: &SpaceSpec,
        time_spec: &TimeSpec,
        num_leading_points: u8,
        num_trailing_points: u8,
        extra_spec: Option<&str>,
    ) -> Result<DataCache, Error>;
}

// TODO: this needs updating when we update the proto
/// Data routing utility for ROVE
///
/// This contains a map of &str to [`DataConnector`]s and is used by ROVE to
/// pull data for QC tests the &str keys in the hashmap correspond to the data
/// source name you would use in the call to validate_series or validate_spatial.
/// So in the example below, you would contruct a spatial_id like "test:single"
/// to direct ROVE to fetch data from `TestDataSource` and pass it the data_id
/// "single"
///
/// ```
/// use rove::{
///     data_switch::{DataConnector, DataSwitch},
///     dev_utils::TestDataSource,
/// };
/// use std::collections::HashMap;
///
/// let data_switch = DataSwitch::new(HashMap::from([
///     ("test", Box::new(TestDataSource{
///         data_len_single: 3,
///         data_len_series: 1000,
///         data_len_spatial: 1000,
///     }) as Box<dyn DataConnector + Send>),
/// ]));
/// ```
#[derive(Debug)]
pub struct DataSwitch<'ds> {
    sources: HashMap<&'ds str, Box<dyn DataConnector + Send>>,
}

impl<'ds> DataSwitch<'ds> {
    /// Instantiate a new DataSwitch
    ///
    /// See the DataSwitch struct documentation for more info
    pub fn new(sources: HashMap<&'ds str, Box<dyn DataConnector + Send>>) -> Self {
        Self { sources }
    }

    // TODO: handle backing sources
    pub(crate) async fn fetch_data(
        &self,
        data_source_id: &str,
        space_spec: &SpaceSpec,
        time_spec: &TimeSpec,
        num_leading_points: u8,
        num_trailing_points: u8,
        extra_spec: Option<&str>,
    ) -> Result<DataCache, Error> {
        let data_source = self
            .sources
            .get(data_source_id)
            .ok_or_else(|| Error::InvalidDataSource(data_source_id.to_string()))?;

        data_source
            .fetch_data(
                space_spec,
                time_spec,
                num_leading_points,
                num_trailing_points,
                extra_spec,
            )
            .await
    }
}
