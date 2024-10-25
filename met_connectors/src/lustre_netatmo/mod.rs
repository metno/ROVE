use async_trait::async_trait;
use chrono::prelude::*;
use chronoutil::RelativeDuration;
use rove::{
    data_switch,
    data_switch::{DataCache, DataConnector, SpaceSpec, TimeSpec, Timeseries, Timestamp},
};
use serde::Deserialize;
use std::{fs::File, io};

#[derive(Debug)]
pub struct LustreNetatmo;

#[derive(Debug, Deserialize)]
struct Record {
    lat: f32,
    lon: f32,
    elev: f32,
    value: f32,
    // Provider ID
    // 1=WMO stations, 2=MET Non-WMO stations, 3=Netatmo, 4=Foreign WMO, 5=SVV, 6=Bergensværet, 7=FMI, 8=Luftambulansen, 9=Holfuy, 100=Radar precipitation
    prid: u32,
    // QC flag
    // 0 = OK, >=l = fail
    dqc: u32,
}

fn read_netatmo(timestamp: Timestamp) -> Result<DataCache, data_switch::Error> {
    // timestamp should be validated before it gets here, so it should be safe to unwrap
    let time = Utc.timestamp_opt(timestamp.0, 0).unwrap();
    // TODO: time resolution might change in the future
    let period = RelativeDuration::hours(1);

    if time.minute() != 0 || time.second() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "timestamps for fetching netatmo data must be on the hour",
        )
        .into());
    }

    let path = format!("{}", time.format("/lustre/storeB/immutable/archive/projects/metproduction/yr_short/%Y/%m/%d/obs_ta_%Y%m%dT%HZ.txt"));

    let file = File::open(path)?;

    // TODO: probably some optimisation potential here?
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    let mut elevs = Vec::new();
    let mut values = Vec::new();

    let mut rdr = csv::ReaderBuilder::new().delimiter(b';').from_reader(file);
    for result in rdr.deserialize() {
        let record: Record = result.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // TODO: should we allow more prids?
        // prid 3 represents netatmo data, but if we use this as a backing set
        // I wonder if there's any harm in adding others
        if record.prid == 3 && record.dqc == 0 {
            lats.push(record.lat);
            lons.push(record.lon);
            elevs.push(record.elev);
            values.push(Timeseries {
                // would be nice if we could come up with better identifiers for this
                tag: format!("({},{})", record.lat, record.lon),
                values: vec![Some(record.value)],
            });
        }
    }

    Ok(DataCache::new(
        values, lats, lons, elevs, timestamp, period, 0, 0,
    ))
}

#[async_trait]
impl DataConnector for LustreNetatmo {
    async fn fetch_data(
        &self,
        space_spec: &SpaceSpec,
        time_spec: &TimeSpec,
        num_leading_points: u8,
        num_trailing_points: u8,
        _extra_spec: Option<&str>,
    ) -> Result<DataCache, data_switch::Error> {
        if num_leading_points != 0
            || num_trailing_points != 0
            || time_spec.timerange.start != time_spec.timerange.end
        {
            return Err(data_switch::Error::UnimplementedSeries(
                "netatmo files are only in timeslice format".to_string(),
            ));
        }

        match space_spec {
            SpaceSpec::All => {
                let start_time = time_spec.timerange.start;
                tokio::task::spawn_blocking(move || read_netatmo(start_time)).await?
            }
            SpaceSpec::One(_) => Err(data_switch::Error::UnimplementedSeries(
                "netatmo files are only in timeslice format".to_string(),
            )),
            // TODO: should we implement this?
            SpaceSpec::Polygon(_) => Err(data_switch::Error::UnimplementedSpatial(
                "this connector cannot filter netatmo files by a polygon".to_string(),
            )),
        }
    }
}

// TODO: add unit test?
