use crate::frost::{util, Error, FrostLatLonElev, FrostObs};
use chrono::{prelude::*, Duration};
use chronoutil::RelativeDuration;
use rove::data_switch::{self, DataCache, Polygon, SpaceSpec, TimeSpec, Timeseries, Timestamp};

use super::Credentials;

#[allow(clippy::type_complexity)]
fn extract_data(
    mut resp: serde_json::Value,
    time: DateTime<Utc>,
    request_time_resolution: RelativeDuration,
) -> Result<Vec<((String, Vec<FrostObs>), FrostLatLonElev)>, Error> {
    let ts_portion = resp
        .get_mut("data")
        .ok_or(Error::FindObs(
            "couldn't find data field on root".to_string(),
        ))?
        .get_mut("tseries")
        .ok_or(Error::FindObs(
            "couldn't find tseries field on data".to_string(),
        ))?
        .as_array_mut()
        .ok_or(Error::FindObs("couldn't get array of tseries".to_string()))?;

    let data = ts_portion
        .iter_mut()
        .map(|ts| {
            let header = ts.get_mut("header").ok_or(Error::FindObs(
                "couldn't find header field on tseries".to_string(),
            ))?;

            // TODO: differentiate actual parse errors from missing duration?
            let ts_time_resolution_result = util::extract_duration(header);
            if ts_time_resolution_result.is_err()
                || ts_time_resolution_result.unwrap() != request_time_resolution
            {
                return Ok(None);
            }

            let station_id = util::extract_station_id(header)?;

            // TODO: Should there be a location for each observation?
            let location = util::extract_location(header, time)?;

            let obs: Vec<FrostObs> = serde_json::from_value(
                ts.get_mut("observations")
                    .ok_or(Error::FindObs(
                        "couldn't find observations field on tseries".to_string(),
                    ))?
                    .take(),
            )?;

            Ok(Some(((station_id, obs), location)))
        })
        .filter_map(Result::transpose)
        .collect::<Result<Vec<((String, Vec<FrostObs>), FrostLatLonElev)>, Error>>()?;

    Ok(data)
}

fn parse_polygon(polygon: &Polygon) -> String {
    let mut s = String::new();
    s.push('[');
    let mut first = true;
    for coord in polygon.iter() {
        if !first {
            s.push(',');
        }
        s.push('{');
        s.push_str((format!("\"lat\":{},\"lon\":{}", coord.lat, coord.lon)).as_str());
        s.push('}');
        first = false;
    }
    s.push(']');
    s
}

fn json_to_data_cache(
    resp: serde_json::Value,
    period: RelativeDuration,
    num_leading_points: u8,
    num_trailing_points: u8,
    interval_start: DateTime<Utc>,
    interval_end: DateTime<Utc>,
) -> Result<DataCache, Error> {
    let ts_vec = extract_data(resp, interval_start, period)?;

    let processed_ts_vec = ts_vec
        .into_iter()
        .map(|((station_id, obses), location)| {
            // TODO: preallocate?
            // let ts_length = (end_time - first_obs_time) / period;
            let mut data = Vec::new();

            let mut curr_obs_time = interval_start - period * i32::from(num_leading_points);
            let first_obs_time = obses
                .first()
                .ok_or(Error::MissingObs(
                    "obs array from frost is empty".to_string(),
                ))?
                .time;

            // handle misalignment of interval_start with ts, and leading missing values
            if curr_obs_time != first_obs_time {
                if first_obs_time < curr_obs_time {
                    return Err(Error::Misalignment(
                        "the first obs returned by frost is outside the time range".to_string(),
                    ));
                }

                while first_obs_time >= curr_obs_time + period {
                    data.push(None);
                    curr_obs_time = curr_obs_time + period;
                }

                if first_obs_time != curr_obs_time + period {
                    return Err(Error::Misalignment(
                        "the first obs returned by frost is not aligned with the start time and period".to_string(),
                    ));
                }

                curr_obs_time = first_obs_time;
            }

            // insert obses into data, with Nones for gaps in the series
            for obs in obses {
                while curr_obs_time < obs.time {
                    data.push(None);
                    curr_obs_time = curr_obs_time + period;
                }
                if curr_obs_time == obs.time {
                    data.push(Some(obs.body.value));
                    curr_obs_time = curr_obs_time + period;
                } else {
                    return Err(Error::Misalignment(
                        "obs misaligned with series".to_string(),
                    ));
                }
            }

            // handle trailing missing values
            while curr_obs_time < interval_end {
                data.push(None);
                curr_obs_time = curr_obs_time + period;
            }

            Ok((Timeseries{tag:station_id, values:data}, location))
        })
        .collect::<Result<Vec<(Timeseries<Option<f32>>, FrostLatLonElev)>, Error>>()?;

    let lats = processed_ts_vec.iter().map(|ts| ts.1.latitude).collect();
    let lons = processed_ts_vec.iter().map(|ts| ts.1.longitude).collect();
    let elevs = processed_ts_vec.iter().map(|ts| ts.1.elevation).collect();
    Ok(DataCache::new(
        processed_ts_vec.into_iter().map(|ts| ts.0).collect(),
        lats,
        lons,
        elevs,
        Timestamp(interval_start.timestamp()),
        period,
        num_leading_points,
        num_trailing_points,
    ))
}

pub async fn fetch_data_inner(
    space_spec: &SpaceSpec,
    time_spec: &TimeSpec,
    num_leading_points: u8,
    num_trailing_points: u8,
    extra_spec: Option<&str>,
    credentials: &Credentials,
) -> Result<DataCache, data_switch::Error> {
    // TODO: figure out how to share the client between rove reqs
    let client = reqwest::Client::new();

    let element_id = extra_spec.ok_or(data_switch::Error::InvalidExtraSpec {
        data_source: "frost",
        extra_spec: extra_spec.map(|s| s.to_string()),
        source: Box::new(Error::InvalidElementId(
            "extra_spec must contain an element id",
        )),
    })?;

    // TODO: should these maybe just be passed in this way?
    let interval_start = Utc.timestamp_opt(time_spec.timerange.start.0, 0).unwrap();
    let interval_end = Utc.timestamp_opt(time_spec.timerange.end.0, 0).unwrap();

    let extra_query_param = match space_spec {
        SpaceSpec::One(station_id) => Ok(("stationids", station_id.to_string())),
        SpaceSpec::Polygon(polygon) => Ok(("polygon", parse_polygon(polygon))),
        SpaceSpec::All => Err(data_switch::Error::Other(Box::new(
            Error::InvalidSpaceSpec("space_spec for frost cannot be `All`, as frost will time out"),
        ))),
    }?;

    let resp: serde_json::Value = client
        .get("https://frost-beta.met.no/api/v1/obs/met.no/filter/get")
        .basic_auth(&credentials.username, Some(&credentials.password))
        .query(&[
            extra_query_param,
            ("elementids", element_id.to_string()),
            ("incobs", "true".to_string()),
            (
                "time",
                format!(
                    "{}/{}",
                    (interval_start - time_spec.time_resolution * i32::from(num_leading_points))
                        .to_rfc3339_opts(SecondsFormat::Secs, true),
                    (interval_end
                        + (time_spec.time_resolution * i32::from(num_trailing_points))
                        + Duration::seconds(1))
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
                ), // .as_str(),
            ),
            ("geopostype", "stationary".to_string()),
        ])
        .send()
        .await
        .map_err(|e| data_switch::Error::Other(Box::new(Error::Request(e))))?
        .json()
        .await
        .map_err(|e| data_switch::Error::Other(Box::new(Error::Request(e))))?;

    // TODO: send this part to rayon?
    json_to_data_cache(
        resp,
        time_spec.time_resolution,
        num_leading_points,
        num_trailing_points,
        interval_start,
        interval_end,
    )
    .map_err(|e| data_switch::Error::Other(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESP_SERIES: &str = r#"
{
  "data": {
    "tstype": "met.no/filter",
    "tseries": [
      {
        "header": {
          "id": {
            "level": 0,
            "parameterid": 211,
            "sensor": 0,
            "stationid": 18700
          },
          "extra": {
            "element": {
              "description": "Air temperature (default 2 m above ground), present value",
              "id": "air_temperature",
              "name": "Air temperature",
              "unit": "degC"
            },
            "station": {
              "location": [
                {
                  "from": "1931-01-01T00:00:00Z",
                  "to": "1940-12-31T00:00:00Z",
                  "value": {
                    "elevation(masl/hs)": "85",
                    "latitude": "59.939200",
                    "longitude": "10.718600"
                  }
                },
                {
                  "from": "1941-01-01T00:00:00Z",
                  "to": "9999-01-01T00:00:00Z",
                  "value": {
                    "elevation(masl/hs)": "94",
                    "latitude": "59.942300",
                    "longitude": "10.720000"
                  }
                }
              ],
              "shortname": "Oslo (Blindern)"
            },
            "timeseries": {
              "geometry": {
                "level": {
                  "unit": "m",
                  "value": "2"
                }
              },
              "quality": {
                "exposure": [
                  {
                    "from": "1931-01-01T06:00:00Z",
                    "to": "1931-01-01T06:00:00Z",
                    "value": "1"
                  }
                ],
                "performance": [
                  {
                    "from": "1937-01-01T06:00:00Z",
                    "to": "1937-01-01T06:00:00Z",
                    "value": "unknown"
                  }
                ]
              },
              "timeoffset": "PT0H",
              "timeresolution": "PT1H"
            }
          },
          "available": {
            "from": "1937-01-01T06:00:00Z"
          }
        },
        "observations": [
          {
            "time": "2023-06-26T12:00:00Z",
            "body": {
              "qualitycode": "0",
              "value": "27.3999996"
            }
          },
          {
            "time": "2023-06-26T13:00:00Z",
            "body": {
              "qualitycode": "0",
              "value": "25.7999992"
            }
          },
          {
            "time": "2023-06-26T14:00:00Z",
            "body": {
              "qualitycode": "0",
              "value": "26"
            }
          }
        ]
      }
    ]
  }
}"#;

    #[test]
    fn test_json_to_series_cache() {
        let resp = serde_json::from_str(RESP_SERIES).unwrap();

        let series_cache = json_to_data_cache(
            resp,
            RelativeDuration::hours(1),
            2,
            0,
            Utc.with_ymd_and_hms(2023, 6, 26, 14, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2023, 6, 26, 14, 0, 0).unwrap(),
        )
        .unwrap();

        assert_eq!(
            Utc.timestamp_opt(series_cache.start_time.0, 0).unwrap(),
            // This was 12 before, but I think it was wrong before, as the start time in the cache
            // should be the timestamp for the first real data point, excluding the leading values
            Utc.with_ymd_and_hms(2023, 6, 26, 14, 0, 0).unwrap(),
        );
        assert_eq!(
            series_cache.data[0].values,
            vec![Some(27.3999996), Some(25.7999992), Some(26.)]
        );
    }

    const RESP_SPATIAL: &str = r#"
{
    "data": {
        "tstype": "met.no/filter",
        "tseries": [
            {
                "header": {
                    "id": {
                        "level": 0,
                        "parameterid": 211,
                        "sensor": 0,
                        "stationid": 18700
                    },
                    "extra": {
                        "element": {},
                        "station": {
                            "location": [
                                {
                                    "from": "1931-01-01T00:00:00Z",
                                    "to": "1940-12-31T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "85",
                                        "latitude": "59.939200",
                                        "longitude": "10.718600"
                                    }
                                },
                                {
                                    "from": "1941-01-01T00:00:00Z",
                                    "to": "9999-01-01T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "94",
                                        "latitude": "59.942300",
                                        "longitude": "10.720000"
                                    }
                                }
                            ],
                            "shortname": "Oslo (Blindern)"
                        },
                        "timeseries": {}
                    },
                    "available": {
                        "from": "1937-01-01T06:00:00Z"
                    }
                },
                "observations": [
                    {
                        "time": "2023-08-13T18:00:00Z",
                        "body": {
                            "qualitycode": "0",
                            "value": "17"
                        }
                    }
                ]
            },
            {
                "header": {
                    "id": {
                        "level": 0,
                        "parameterid": 211,
                        "sensor": 0,
                        "stationid": 18315
                    },
                    "extra": {
                        "element": {
                            "description": "Air temperature (default 2 m above ground), present value",
                            "id": "air_temperature",
                            "name": "Air temperature",
                            "unit": "degC"
                        },
                        "station": {
                            "location": [
                                {
                                    "from": "2016-01-07T00:00:00Z",
                                    "to": "9999-01-01T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "37",
                                        "latitude": "59.919000",
                                        "longitude": "10.762300"
                                    }
                                }
                            ],
                            "shortname": "Sofienberg "
                        },
                        "timeseries": {
                            "geometry": {
                                "level": {
                                    "unit": "m",
                                    "value": "2"
                                }
                            },
                            "quality": {
                                "exposure": [
                                    {
                                        "from": "2016-01-07T00:00:00Z",
                                        "to": "2016-01-07T00:00:00Z",
                                        "value": "5"
                                    },
                                    {
                                        "from": "2015-07-07T22:00:00Z",
                                        "to": "2015-07-07T22:00:00Z",
                                        "value": "unknown"
                                    }
                                ],
                                "performance": [
                                    {
                                        "from": "2016-01-07T00:00:00Z",
                                        "to": "2016-01-07T00:00:00Z",
                                        "value": "E"
                                    },
                                    {
                                        "from": "2015-07-07T22:00:00Z",
                                        "to": "2015-07-07T22:00:00Z",
                                        "value": "unknown"
                                    }
                                ]
                            },
                            "timeoffset": "PT0H",
                            "timeresolution": "PT1H"
                        }
                    },
                    "available": {
                        "from": "2015-07-07T22:00:00Z"
                    }
                },
                "observations": [
                    {
                        "time": "2023-08-13T18:00:00Z",
                        "body": {
                            "qualitycode": "0",
                            "value": "18.1000004"
                        }
                    }
                ]
            },
            {
                "header": {
                    "id": {
                        "level": 0,
                        "parameterid": 211,
                        "sensor": 0,
                        "stationid": 18950
                    },
                    "extra": {
                        "element": {
                            "description": "Air temperature (default 2 m above ground), present value",
                            "id": "air_temperature",
                            "name": "Air temperature",
                            "unit": "degC"
                        },
                        "station": {
                            "location": [
                                {
                                    "from": "1927-08-01T00:00:00Z",
                                    "to": "1975-12-01T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "514",
                                        "latitude": "59.984700",
                                        "longitude": "10.669300"
                                    }
                                },
                                {
                                    "from": "1975-12-01T00:00:00Z",
                                    "to": "1976-12-31T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "514",
                                        "latitude": "59.984700",
                                        "longitude": "10.669300"
                                    }
                                },
                                {
                                    "from": "1997-09-16T00:00:00Z",
                                    "to": "2012-01-04T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "514",
                                        "latitude": "59.984700",
                                        "longitude": "10.669300"
                                    }
                                },
                                {
                                    "from": "2012-01-04T00:00:00Z",
                                    "to": "9999-01-01T00:00:00Z",
                                    "value": {
                                        "elevation(masl/hs)": "514",
                                        "latitude": "59.984700",
                                        "longitude": "10.669300"
                                    }
                                }
                            ],
                            "shortname": "Tryvannshøgda"
                        },
                        "timeseries": {
                            "geometry": {
                                "level": {
                                    "unit": "m",
                                    "value": "2"
                                }
                            },
                            "quality": {
                                "exposure": [
                                    {
                                        "from": "1943-01-01T06:00:00Z",
                                        "to": "1943-01-01T06:00:00Z",
                                        "value": "1"
                                    },
                                    {
                                        "from": "1975-12-31T18:00:00Z",
                                        "to": "1975-12-31T18:00:00Z",
                                        "value": "unknown"
                                    },
                                    {
                                        "from": "1997-09-16T00:00:00Z",
                                        "to": "1997-09-16T00:00:00Z",
                                        "value": "1"
                                    }
                                ],
                                "performance": [
                                    {
                                        "from": "1943-01-01T06:00:00Z",
                                        "to": "1943-01-01T06:00:00Z",
                                        "value": "unknown"
                                    }
                                ]
                            },
                            "timeoffset": "PT0H",
                            "timeresolution": "PT1H"
                        }
                    },
                    "available": {
                        "from": "1943-01-01T06:00:00Z"
                    }
                },
                "observations": [
                    {
                        "time": "2023-08-13T18:00:00Z",
                        "body": {
                            "qualitycode": "0",
                            "value": "13.3000002"
                        }
                    }
                ]
            }
        ]
    }
}"#;

    #[test]
    fn test_json_to_spatial_cache() {
        let resp = serde_json::from_str(RESP_SPATIAL).unwrap();

        let spatial_cache = json_to_data_cache(
            resp,
            RelativeDuration::hours(1),
            0,
            0,
            Utc.with_ymd_and_hms(2023, 8, 13, 18, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2023, 8, 13, 18, 0, 0).unwrap(),
        )
        .unwrap();

        // This test is a lot less useful since we made spatial queries only return timeseries with
        // the requested timeresolution
        assert_eq!(spatial_cache.data.len(), 2);
    }
}
