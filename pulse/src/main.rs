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

use controller::Controller;

/// Main entry point for the Pulse application
///
/// This function:
/// 1. Initializes the logging system
/// 2. Creates and starts the controller
/// 3. Begins the URL monitoring process
#[tokio::main]
async fn main() {
    logger::init();

    // console_subscriber::init();
    let controller = Controller::new();
    controller.start().await;
}
