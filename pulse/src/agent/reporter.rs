use crate::message::Endpoint;
use crate::task::Runnable;
use async_trait::async_trait;
use log::trace;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

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
    result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
}

impl Reporter {
    /// Creates a new Reporter instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this reporter
    /// * `result_receiver` - Channel for receiving results from workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    pub fn new(id: usize, result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>) -> Self {
        Self {
            name: format!("{}-{}", REPORTER_NAME_PREFIX, id),
            result_receiver,
        }
    }

    /// Continuously receives and processes results from workers
    async fn process_results(&self) {
        while let Some(endpoint) = self.receive_result().await {
            let retry_count = endpoint.request.retry_count as usize;
            let current_attempts = endpoint.result.len();
            trace!(
                "Final result for URL: {} - Attempts: {}/{}",
                endpoint.request.url, current_attempts, retry_count
            );
            trace!("{:?}", endpoint);
        }
    }

    async fn receive_result(&self) -> Option<Endpoint> {
        let mut receiver = self.result_receiver.lock().await;
        receiver.recv().await
    }
}

#[async_trait]
impl Runnable for Reporter {
    async fn start(&self) {
        self.process_results().await;
    }

    fn name(&self) -> &str {
        &self.name
    }
}
