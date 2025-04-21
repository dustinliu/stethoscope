use crate::config::{self, Config};
use crate::runnable::Runnable;
use crate::{broker::Broker, message::Endpoint};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::error::SendError;
use tokio::time::{Duration, MissedTickBehavior, interval};

/// Prefix for dispatcher instance names
const DISPATCHER_NAME_PREFIX: &str = "Dispatcher";

/// Dispatcher responsible for periodically generating and sending Endpoints to workers via the Broker.
///
/// The Dispatcher component:
/// - Periodically (based on `config.dispatcher.check_interval`) generates a list of `Endpoint`s.
/// - Sends these `Endpoint`s to the workers through the `Broker`'s endpoint channel.
/// - Handles potential send errors and shutdown signals gracefully.
///
/// # Fields
/// * `name` - Name of the dispatcher instance (e.g., "Dispatcher-0").
/// * `broker` - Cloned `Broker` instance for communication.
/// * `config` - Static reference to the application configuration.
pub struct Dispatcher {
    name: String,
    broker: Broker,
    config: &'static Config,
}

impl Dispatcher {
    /// Creates a new Dispatcher instance.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this dispatcher instance.
    /// * `broker` - Cloned `Broker` instance for communication.
    ///
    /// # Returns
    /// A new Dispatcher instance.
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", DISPATCHER_NAME_PREFIX, id),
            broker,
            config: config::instance(),
        }
    }

    /// Generates a list of endpoints for monitoring.
    ///
    /// Currently, this function creates a fixed set of test endpoints pointing to localhost.
    /// TODO: Replace this with logic to fetch endpoints from a configuration source or database.
    ///
    /// # Returns
    /// A vector of `Endpoint` instances.
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

    /// Generates endpoints and sends them to the broker's endpoint channel.
    ///
    /// Called periodically by the `run` method.
    /// It retrieves endpoints using `gen_urls` and attempts to send each one
    /// using `broker.send_endpoint`.
    /// Handles `SendError` by logging a warning and continuing.
    /// Returns `Err` if the broker signals shutdown during sending, allowing the
    /// `run` loop to terminate gracefully.
    async fn dispatch_urls(&self) -> Result<()> {
        for url in Self::gen_urls() {
            // Use if let to handle only the error case
            if let Err(e) = self.broker.send_endpoint(url).await {
                // Try to downcast the error to SendError<Endpoint>
                if e.downcast_ref::<SendError<Endpoint>>().is_some() {
                    // It's a regular send error (e.g., channel closed unexpectedly)
                    tracing::warn!("Failed to send URL, skipping: {}", e);
                    // Continue to the next URL in the loop
                } else {
                    // Could not downcast, assume it's the shutdown error from anyhow::bail!
                    // Propagate the shutdown error
                    tracing::debug!("Detected shutdown signal via error type: {}", e);
                    return Err(e);
                }
            }
            // If Ok, do nothing and continue to the next URL
        }
        // All URLs processed without shutdown
        Ok(())
    }
}

#[async_trait]
impl Runnable for Dispatcher {
    // The main loop for the dispatcher.
    // Runs periodically based on `config.dispatcher.check_interval`.
    // In each iteration, it calls `dispatch_urls` to generate and send endpoints.
    // If `dispatch_urls` returns an error (indicating shutdown), the loop breaks.
    async fn run(&mut self) {
        tracing::info!("Starting dispatcher: {}", self.name);
        let mut interval = interval(self.config.dispatcher.check_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            // Call dispatch_urls and check for shutdown error
            if let Err(e) = self.dispatch_urls().await {
                tracing::info!("Dispatcher {} received shutdown signal: {}", self.name, e);
                break; // Exit the loop on shutdown error
            }
            // Otherwise, continue the loop
        }
        tracing::info!("Dispatcher {} stopped.", self.name);
    }

    /// Returns the name of this dispatcher instance.
    fn name(&self) -> &str {
        &self.name
    }
}
