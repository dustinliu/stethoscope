use crate::config::{self, Config};
use crate::runnable::Runnable;
use crate::{broker::Broker, message::Endpoint};
use async_trait::async_trait;
use log::{debug, warn};
use tokio::time::{Duration, interval};

/// Prefix for dispatcher instance names
const DISPATCHER_NAME_PREFIX: &str = "Dispatcher";

/// Dispatcher responsible for generating and sending URLs to workers
///
/// The Dispatcher component:
/// - Generates URLs for monitoring
/// - Dispatches URLs to workers through channels
/// - Manages the URL generation lifecycle
///
/// # Fields
/// * `name` - Name of the dispatcher instance
/// * `url_sender` - Channel for sending URLs to workers
/// * `shutdown_receiver` - Receiver for shutdown signals
pub struct Dispatcher {
    name: String,
    broker: Broker,
    config: &'static Config,
}

impl Dispatcher {
    /// Creates a new Dispatcher instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this dispatcher
    /// * `url_sender` - Channel for sending URLs to workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    ///
    /// # Returns
    /// A new Dispatcher instance with the specified configuration
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", DISPATCHER_NAME_PREFIX, id),
            broker,
            config: config::instance(),
        }
    }

    /// Generates a list of endpoints for monitoring
    ///
    /// This function creates a set of test endpoints for demonstration purposes.
    /// In a production environment, this would be replaced with actual URLs to monitor.
    ///
    /// # Returns
    /// A vector of Endpoint instances configured for monitoring
    fn gen_urls() -> Vec<Endpoint> {
        (0..=100)
            .map(|i| Endpoint {
                id: i,
                url: String::from("http://localhost"),
                timeout: Duration::from_secs(5),
                failure_threshold: 3,
            })
            .collect()
    }

    /// Continuously generates and sends URLs to workers
    ///
    /// This method:
    /// 1. Creates an interval timer for URL generation
    /// 2. Generates and sends URLs at each interval
    /// 3. Monitors for shutdown signals
    async fn dispatch_urls(&self) {
        let mut interval = interval(self.config.check_interval());
        interval.tick().await;
        for url in Self::gen_urls() {
            if let Err(e) = self.broker.send_endpoint(url).await {
                warn!("Failed to send URL: {}", e);
            }
        }
    }
}

#[async_trait]
impl Runnable for Dispatcher {
    async fn run(&self) {
        let mut broker = self.broker.clone();
        loop {
            tokio::select! {
                _ = broker.wait_for_shutdown() => {
                    break;
                }
                _ = self.dispatch_urls() => {}
            }
        }
    }

    /// Returns the name of this dispatcher instance
    fn name(&self) -> &str {
        &self.name
    }
}
