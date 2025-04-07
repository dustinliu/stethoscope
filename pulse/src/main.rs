mod agent;
mod config;
mod controller;
mod logger;
mod message;
mod task;
mod worker;

use controller::Controller;

/// Main entry point for the Pulse application
///
/// Initializes the controller and starts the URL monitoring system
#[tokio::main]
async fn main() {
    logger::init_logger();

    let mut controller = Controller::new();
    controller.start().await;
}
