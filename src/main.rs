use env_logger::Builder;
use log::{info, LevelFilter};
use std::env;
use std::fs::OpenOptions;
use std::io::Write;

mod async_client;
mod cache;
mod cli;
mod config;
mod error;
mod id_utils;
mod library;
mod player;
mod search;
mod tui;

use cli::Cli;
use error::DabResult;

#[tokio::main]
async fn main() -> DabResult<()> {
    init_logging();

    let cli = Cli::parse();
    info!("Dab music player starting");

    cli.run().await
}

fn init_logging() {
    let log_file_path = "/tmp/dab_rs.log";

    // Create or open the log file
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path)
        .expect("Failed to open log file");

    let log_level = env::var("DAB_LOG_LEVEL")
        .unwrap_or_else(|_| "info".to_string())
        .to_lowercase();

    let level_filter = match log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    };

    Builder::from_default_env()
        .target(env_logger::Target::Pipe(Box::new(log_file)))
        .filter_level(level_filter)
        .format(|buf, record| {
            writeln!(
                buf,
                "[{}] [{}] [{}:{}] {}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
                record.level(),
                record.module_path().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .init();

    info!("Logging initialized to {}", log_file_path);
}
