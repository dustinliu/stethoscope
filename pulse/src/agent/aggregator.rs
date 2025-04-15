use crate::{agent::Runnable, broker::Broker};
use async_trait::async_trait;
use log::trace;

/// Prefix for aggregator instance names
const AGGREGATOR_NAME_PREFIX: &str = "Aggregator";

/// Aggregator responsible for processing monitoring results
///
/// The Aggregator component:
/// - Receives results from workers
/// - Processes results and handles retries
/// - Manages the result processing lifecycle
///
/// # Fields
/// * `name` - Name of the aggregator instance
/// * `result_receiver` - Channel for receiving results from workers
/// * `shutdown_receiver` - Receiver for shutdown signals
pub struct Aggregator {
    name: String,
    broker: Broker,
}

impl Aggregator {
    /// Creates a new Aggregator instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this aggregator
    /// * `result_receiver` - Channel for receiving results from workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", AGGREGATOR_NAME_PREFIX, id),
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
impl Runnable for Aggregator {
    async fn run(&self) {
        self.process_results().await;
    }

    /// Returns the name of this aggregator instance
    fn name(&self) -> &str {
        &self.name
    }
}
