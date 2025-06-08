mod config;
mod endpoint_provider;
mod logger;
mod message;
mod pulse;
mod reporters;
mod worker_pool;

pub use config::Config;
pub use endpoint_provider::EndpointProvider;
pub use worker_pool::WorkerPool;

use clap::Parser;
use std::path::PathBuf;

use pulse::Pulse;

/// Define command line arguments using clap
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "config file", env = "pulse_config")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let _logger = logger::init();

    let cli = Cli::parse();
    tracing::debug!("Config path: {:?}", cli.config);
    let conf = Config::new(&cli.config).unwrap_or_else(|e| {
        eprintln!("Failed to load configuration: {:?}", e);
        std::process::exit(1);
    });

    Pulse::new(conf).start().await
}
