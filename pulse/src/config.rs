/// Configuration module for the Pulse URL monitoring system
///
/// This module manages the global configuration settings for the application,
/// including worker counts, agent counts, and timing intervals.
use std::sync::OnceLock;
use tokio::time::Duration;

/// Global configuration settings for the application
///
/// This struct holds all configurable parameters that affect the
/// behavior of the URL monitoring system.
///
/// # Fields
/// * `worker_num` - Number of worker threads for processing URLs
/// * `aggregator_num` - Number of agent threads for managing workers
/// * `check_interval` - Duration between health checks
#[derive(Debug)]
pub struct Config {
    worker_num: usize,
    check_interval: Duration,
}

/// Global singleton instance of the configuration
static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    /// Creates a new Config instance with default values
    ///
    /// # Returns
    /// A new Config instance with the following defaults:
    /// * 50 worker threads
    /// * 1 agent thread
    /// * 5 second check interval
    fn new() -> Self {
        Config {
            worker_num: 20,
            check_interval: Duration::from_secs(1),
        }
    }

    /// Returns the number of worker threads
    pub fn worker_num(&self) -> usize {
        self.worker_num
    }

    /// Returns the duration between health checks
    ///
    /// TODO: Change the default value to 5 minutes
    pub fn check_interval(&self) -> Duration {
        self.check_interval
    }

    pub fn alert_buffer_len(&self) -> usize {
        100
    }

    pub fn enable_stdio_reporter(&self) -> bool {
        false
    }
}

/// Returns the global singleton instance of Config
///
/// This function ensures that only one Config instance exists
/// throughout the application's lifecycle.
pub fn instance() -> &'static Config {
    INSTANCE.get_or_init(Config::new)
}
