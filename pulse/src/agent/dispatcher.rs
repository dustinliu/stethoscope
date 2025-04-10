/// Dispatcher module for the Pulse URL monitoring system
///
/// This module implements the Dispatcher component which is responsible for
/// generating and distributing URL monitoring tasks to workers.
use crate::{config, message::Endpoint, task::Runnable};
use async_trait::async_trait;
use log::warn;
use std::collections::HashMap;
use tokio::{
    sync::RwLock,
    sync::{mpsc, watch},
    time,
};

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
    url_sender: mpsc::Sender<Endpoint>,
    shutdown_receiver: watch::Receiver<bool>,
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
    pub fn new(
        id: usize,
        url_sender: mpsc::Sender<Endpoint>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> Self {
        Self {
            name: format!("{}-{}", DISPATCHER_NAME_PREFIX, id),
            url_sender,
            shutdown_receiver,
            // endpoints_state: RwLock::new(HashMap::new()),
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
                timeout: time::Duration::from_secs(5),
                retry_delay: time::Duration::from_secs(5),
                retry_count: 3,
            })
            .collect()
    }

    /// Continuously generates and sends URLs to workers
    ///
    /// This method:
    /// 1. Creates an interval timer for URL generation
    /// 2. Generates and sends URLs at each interval
    /// 3. Monitors for shutdown signals
    async fn dispatch_urls(&mut self) {
        let mut interval = time::interval(time::Duration::from_secs(5));
        let mut shutdown = *self.shutdown_receiver.borrow();
        while !shutdown {
            interval.tick().await;
            for url in Self::gen_urls() {
                if let Err(e) = self.url_sender.send(url).await {
                    warn!("Failed to send URL: {}", e);
                }
            }
            match time::timeout(
                config::instance().check_interval(),
                self.shutdown_receiver.changed(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    shutdown = *self.shutdown_receiver.borrow_and_update();
                }
                Ok(Err(e)) => {
                    warn!("Error receiving shutdown signal: {}", e);
                }
                Err(_) => {}
            }
        }
    }
}

#[async_trait]
impl Runnable for Dispatcher {
    /// Starts the dispatcher's URL generation and distribution process
    async fn start(&self) {
        let mut dispatcher = Self {
            name: self.name.clone(),
            url_sender: self.url_sender.clone(),
            shutdown_receiver: self.shutdown_receiver.clone(),
        };
        dispatcher.dispatch_urls().await;
    }

    /// Returns the name of this dispatcher instance
    fn name(&self) -> &str {
        &self.name
    }
}
