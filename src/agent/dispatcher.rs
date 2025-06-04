use std::sync::Weak;

use crate::broker::EndpointSender;
use crate::config::{self, Config};
use crate::runnable::Runnable;
use crate::{broker::Broker, message::Endpoint};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rand::Rng;
use tokio::sync::watch;
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
    sender: EndpointSender,
    shutdown_rx: watch::Receiver<bool>,
    config: &'static Config,
    shutdown_flag: bool,
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
    pub fn new(
        id: usize,
        broker: Weak<Broker>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        match broker.upgrade() {
            Some(broker) => Ok(Self {
                name: format!("{}-{}", DISPATCHER_NAME_PREFIX, id),
                sender: broker.endpoint_sender(),
                shutdown_rx,
                config: config::instance(),
                shutdown_flag: false,
            }),
            None => Err(anyhow::anyhow!("Broker has been dropped or is invalid")),
        }
    }

    /// Generates a list of endpoints for monitoring.
    ///
    /// Creates endpoints that connect to httpbin.org with random delays between 50ms and 200ms.
    ///
    /// # Returns
    /// A vector of `Endpoint` instances.
    fn gen_urls() -> Vec<Endpoint> {
        let mut rng = rand::rng(); // Reverted back to rand::rng()
        (0..=100)
            .map(|i| {
                let delay = rng.random_range(50..=200) as f64 / 1000.0; // Random delay between 50ms to 200ms
                Endpoint {
                    id: i,
                    url: format!("http://httpbin.org/delay/{}", delay),
                    timeout: Duration::from_secs(5),
                    failure_threshold: 3,
                }
            })
            .collect()
    }

    async fn dispatch_urls(&mut self) -> Result<()> {
        for url in Self::gen_urls() {
            tokio::select! {
                // Attempt to send the endpoint
                res = self.sender.send(url) => {
                    if let Err(send_error) = res {
                        // Log the error. SendError usually means the receiver is dropped (channel closed).
                        tracing::warn!("Dispatcher {} failed to send message: {}", self.name, send_error);
                        // Treat channel closed as a terminal error for dispatching.
                        // Convert SendError into anyhow::Error for consistent error handling.
                        // `send_error` itself implements `std::error::Error`, so we can call context() directly.
                        return Err(send_error).context("Endpoint channel closed");
                    }
                }
                // Check for shutdown signal
                _ = self.shutdown_rx.changed() => {
                    tracing::debug!("Dispatcher {} interrupted during dispatch by shutdown signal", self.name);
                    self.shutdown_flag = true;
                    return Ok(());
                }
            }
        }
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
        let mut interval = interval(self.config.dispatcher.check_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            if self.shutdown_flag {
                break;
            } else {
                tokio::select! {
                        _ = interval.tick() => {
                            if let Err(e) = self.dispatch_urls().await {
                            tracing::warn!("Dispatcher {} send message failed: {}", self.name, e);
                        }
                    }
                    _ = self.shutdown_rx.changed() => {
                        tracing::trace!("Dispatcher {} received shutdown signal", self.name);
                        self.shutdown_flag = true;
                        break;
                    }
                }
            }
        }
        tracing::trace!("Dispatcher {} stopped.", self.name);
    }

    /// Returns the name of this dispatcher instance.
    fn name(&self) -> &str {
        &self.name
    }
}
