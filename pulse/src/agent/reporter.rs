use crate::{broker::Broker, runnable::Runnable};
use async_trait::async_trait;
use log::trace;

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
    broker: Broker,
}

impl Reporter {
    /// Creates a new Reporter instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this reporter
    /// * `result_receiver` - Channel for receiving results from workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", REPORTER_NAME_PREFIX, id),
            broker,
        }
    }

    /// Continuously receives and processes results from workers
    ///
    /// This method:
    /// 1. Receives results from the worker channel
    /// 2. Processes each result and logs the outcome
    async fn process_results(&self) {
        if let Some(result) = self.broker.receive_result().await {
            trace!("{:?}", result);
        }
    }
}

#[async_trait]
impl Runnable for Reporter {
    async fn run(&self) {
        self.process_results().await;
    }

    /// Returns the name of this reporter instance
    fn name(&self) -> &str {
        &self.name
    }
}
