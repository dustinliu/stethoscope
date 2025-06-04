/// Main module for the Pulse URL monitoring system
///
/// This module serves as the entry point for the application and initializes
/// all necessary components for URL monitoring functionality.
mod agent;
mod broker;
mod config;
mod controller;
mod logger;
mod message;
mod runnable;

use clap::Parser;
use config::Config;
use std::path::PathBuf;

pub use broker::Broker;

use controller::Controller;

/// Define command line arguments using clap
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", env = "pulse_config")]
    config: PathBuf,
}

/// Main entry point for the Pulse application
///
/// This function:
/// 1. Parses command line arguments
/// 2. Initializes the logging system
/// 3. Creates and starts the controller (passing the config path)
/// 4. Begins the URL monitoring process
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let _logger = logger::init();

    let cli = Cli::parse();
    tracing::debug!("Config path: {:?}", cli.config);
    if let Err(e) = Config::init(&cli.config) {
        eprintln!("Failed to initialize configuration: {:?}", e);
        std::process::exit(1);
    }

    // TODO: Pass cli.config to Controller::new() when implemented
    Controller::new().start().await
}
