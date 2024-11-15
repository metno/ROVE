use clap::Parser;
use met_connectors::LustreNetatmo;
use met_connectors::{frost, Frost};
use rove::{
    data_switch::{DataConnector, DataSwitch},
    load_pipelines, start_server,
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
    #[arg(short, long, default_value_t = String::from("sample_pipelines/fresh"))]
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

    start_server(
        args.address.parse()?,
        data_switch,
        load_pipelines(Path::new(&args.pipeline_dir))?,
    )
    .await
}
