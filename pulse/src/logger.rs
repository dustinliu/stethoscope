/// Logger module for the Pulse URL monitoring system
///
/// This module initializes and configures the application's logging system,
/// setting up appropriate log levels and output formats.
use env_logger::Builder;
use log::LevelFilter;

use crate::config;

/// Initializes the logging system with appropriate configuration
///
/// This function:
/// 1. Sets up the logger with Debug level for the pulse module
/// 2. Parses environment variables for additional configuration
/// 3. Initializes the logger with error handling
pub fn init_logger() {
    let level = if config::instance().debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let mut builder = Builder::new();
    builder
        .filter_module("pulse", level)
        .parse_default_env()
        .try_init()
        .expect("Failed to initialize logger");
}
