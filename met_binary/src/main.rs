use chrono::{TimeZone, Utc};
use chronoutil::RelativeDuration;
use clap::Parser;
use met_connectors::LustreNetatmo;
use met_connectors::{frost, Frost};
use rove::Scheduler;
use rove::{
    data_switch::{DataConnector, DataSwitch, SpaceSpec, TimeSpec, Timestamp},
    load_pipelines, //start_server,
};
use std::{collections::HashMap, path::Path};
use tracing::Level;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = String::from("[::1]:1337"))]
    address: String,
    #[arg(short = 'l', long, default_value_t = Level::INFO)]
    max_trace_level: Level,
    #[arg(short, long, default_value_t = String::from("../sample_pipelines/fresh"))]
    pipeline_dir: String,
    #[arg(long, default_value_t = String::from(""))]
    frost_username: String,
    #[arg(long, default_value_t = String::from(""))]
    frost_password: String,
}

// TODO: use anyhow for error handling?
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_max_level(args.max_trace_level)
        .init();

    let frost_connector = Frost {
        credentials: frost::Credentials {
            username: args.frost_username,
            password: args.frost_password,
        },
    };

    let data_switch = DataSwitch::new(HashMap::from([
        (
            "frost",
            Box::new(frost_connector) as Box<dyn DataConnector + Send>,
        ),
        (
            "lustre_netatmo",
            Box::new(LustreNetatmo) as Box<dyn DataConnector + Send>,
        ),
    ]));

    // start_server(
    //     args.address.parse()?,
    //     data_switch,
    //     load_pipelines(Path::new(&args.pipeline_dir))?,
    // )
    // .await

    let rove_scheduler =
        Scheduler::new(load_pipelines(Path::new(&args.pipeline_dir))?, data_switch);

    let backing_sources: [&str; 0] = [];

    let timespec = TimeSpec::new(
        Timestamp(
            Utc.with_ymd_and_hms(2023, 6, 26, 12, 0, 0)
                .unwrap()
                .timestamp(),
        ),
        Timestamp(
            Utc.with_ymd_and_hms(2023, 6, 26, 14, 0, 0)
                .unwrap()
                .timestamp(),
        ),
        RelativeDuration::hours(1),
    );
    let spacespec = SpaceSpec::One(String::from("18700"));
    let result = rove_scheduler
        .validate_direct(
            "frost",
            &backing_sources,
            &timespec,
            &spacespec,
            "TA_PT1H",
            Some("air_temperature"),
        )
        .await;

    println!("result: {:#?}", result);

    Ok(())
}
