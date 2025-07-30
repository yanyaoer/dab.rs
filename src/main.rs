use std::env;
use log::info;

mod cli;
mod player;
mod library;
mod cache;
mod tui;
mod error;
mod config;

use cli::Cli;
use error::DabResult;

#[tokio::main]
async fn main() -> DabResult<()> {
    env_logger::init();
    
    let cli = Cli::parse();
    info!("Dab music player starting");
    
    cli.run().await
}