/// Reporter module for the Pulse URL monitoring system
///
/// This module implements the Reporter component which is responsible for
/// processing and reporting the results of URL monitoring tasks.
use crate::message::EndpointStatus;
use crate::task::Runnable;
use async_trait::async_trait;
use log::trace;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

/// Prefix for reporter instance names
const REPORTER_NAME_PREFIX: &str = "Reporter";

/// Reporter responsible for processing monitoring results
///
/// The Reporter component:
/// - Receives results from workers
/// - Processes results and handles retries
/// - Manages the result processing lifecycle
///
/// # Fields
/// * `name` - Name of the reporter instance
/// * `result_receiver` - Channel for receiving results from workers
/// * `shutdown_receiver` - Receiver for shutdown signals
pub struct Reporter {
    name: String,
    result_receiver: Arc<Mutex<mpsc::Receiver<EndpointStatus>>>,
}

impl Reporter {
    /// Creates a new Reporter instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this reporter
    /// * `result_receiver` - Channel for receiving results from workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    pub fn new(id: usize, result_receiver: Arc<Mutex<mpsc::Receiver<EndpointStatus>>>) -> Self {
        Self {
            name: format!("{}-{}", REPORTER_NAME_PREFIX, id),
            result_receiver,
        }
    }

    /// Continuously receives and processes results from workers
    ///
    /// This method:
    /// 1. Receives results from the worker channel
    /// 2. Processes each result and logs the outcome
    /// 3. Handles retry logic if needed
    async fn process_results(&self) {
        while let Some(endpoint) = self.receive_result().await {
            let retry_count = endpoint.request.retry_count as usize;
            let current_attempts = endpoint.results.len();
            trace!(
                "Final result for URL: {} - Attempts: {}/{}",
                endpoint.request.url, current_attempts, retry_count
            );
            trace!("{:?}", endpoint);
        }
    }

    /// Receives a single result from the worker channel
    ///
    /// # Returns
    /// An Option containing the received Endpoint if available
    async fn receive_result(&self) -> Option<EndpointStatus> {
        let mut receiver = self.result_receiver.lock().await;
        receiver.recv().await
    }
}

#[async_trait]
impl Runnable for Reporter {
    /// Starts the reporter's result processing loop
    async fn start(&self) {
        self.process_results().await;
    }

    /// Returns the name of this reporter instance
    fn name(&self) -> &str {
        &self.name
    }
}
