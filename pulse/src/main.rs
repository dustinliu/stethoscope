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

pub use broker::Broker;

use controller::Controller;

/// Main entry point for the Pulse application
///
/// This function:
/// 1. Initializes the logging system
/// 2. Creates and starts the controller
/// 3. Begins the URL monitoring process
#[tokio::main]
async fn main() {
    let _logger = logger::init();

    Controller::new().start().await
}
