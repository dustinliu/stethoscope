/// Agent module for the Pulse URL monitoring system
///
/// This module implements the Agent component which is responsible for:
/// 1. Generating and dispatching URLs to workers for monitoring
/// 2. Processing results from workers and handling retries for failed requests
use crate::message::{Endpoint, Request};
use crate::task::Runnable;
use async_trait::async_trait;
use log::{debug, info, trace, warn};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, mpsc};
use tokio::time;

const AGENT_NAME_PREFIX: &str = "Agent";
/// Agent responsible for URL monitoring coordination
///
/// The Agent manages the flow of URL monitoring requests and results:
/// - Dispatches URLs to workers for monitoring
/// - Processes results from workers
/// - Handles retries for failed requests
///
/// # Fields
/// * `sender` - Channel for sending URLs to workers
/// * `receiver` - Channel for receiving results from workers, wrapped in Arc<Mutex> for thread safety
/// * `shutdown_flag` - Flag to indicate if the agent should shut down
pub struct Agent {
    name: String,
    url_sender: mpsc::Sender<Endpoint>,
    result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    shutdown_flag: Arc<AtomicBool>,
}

impl Agent {
    /// Creates a new Agent instance
    ///
    /// # Arguments
    ///
    /// * `sender` - The sender channel for sending URLs to workers
    /// * `receiver` - The receiver channel wrapped in Arc<Mutex> for receiving results from workers
    /// * `shutdown_flag` - A flag to indicate if the agent should shut down
    ///
    /// # Returns
    ///
    /// A new Agent instance
    pub fn new(
        id: usize,
        url_sender: mpsc::Sender<Endpoint>,
        result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
        shutdown_flag: Arc<AtomicBool>,
    ) -> Self {
        Agent {
            name: format!("{}-{}", AGENT_NAME_PREFIX, id),
            url_sender,
            result_receiver,
            shutdown_flag,
        }
    }

    /// Continuously generates and sends URLs to workers
    async fn dispatch_urls(url_sender: mpsc::Sender<Endpoint>, shutdown_flag: Arc<AtomicBool>) {
        let mut interval = time::interval(time::Duration::from_secs(5));
        while !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
            interval.tick().await;
            for url in gen_urls() {
                if let Err(e) = url_sender.send(url).await {
                    warn!("Failed to send URL: {}", e);
                }
            }
        }
        info!("URL dispatcher shutdown complete");
    }

    /// Continuously receives and processes results from workers
    async fn process_results(
        url_sender: mpsc::Sender<Endpoint>,
        result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    ) {
        while let Some(endpoint) = {
            let mut receiver_guard = result_receiver.lock().await;
            receiver_guard.recv().await
        } {
            // Check if the last result is a failure and if we need to retry
            let is_failure = endpoint.result.last().map_or_else(
                || false,
                |r| r.status.is_client_error() || r.status.is_server_error(),
            );

            let retry_count = endpoint.request.retry_count as usize;
            let current_attempts = endpoint.result.len();

            if is_failure && current_attempts < retry_count {
                // If the last attempt failed and we haven't reached retry_count,
                // wait for retry_delay and send it back to be processed again
                trace!(
                    "Attempt {} failed for URL: {}. Retrying in {:?}...",
                    current_attempts, endpoint.request.url, endpoint.request.retry_delay
                );
                trace!("request: {:?}", endpoint.request);

                time::sleep(endpoint.request.retry_delay).await;
                if let Err(e) = url_sender.send(endpoint).await {
                    warn!("Failed to send retry URL: {}", e);
                }
            } else {
                // Either success or reached retry_count, print the result
                trace!(
                    "Final result for URL: {} - Attempts: {}/{}",
                    endpoint.request.url, current_attempts, retry_count
                );
            }
        }
        info!("Result processor shutting down...");
    }
}

/// Generates a list of endpoints for monitoring
///
/// This function creates a set of test endpoints for demonstration purposes.
/// In a production environment, this would be replaced with actual URLs to monitor.
///
/// # Returns
///
/// A vector of Endpoint instances with default configuration
fn gen_urls() -> Vec<Endpoint> {
    (0..=100)
        .map(|_| Endpoint {
            request: Request {
                url: String::from("http://localhost"),
                timeout: time::Duration::from_secs(5),
                retry_delay: time::Duration::from_secs(5),
                retry_count: 3,
            },
            result: Vec::new(),
        })
        .collect()
}

#[async_trait]
impl Runnable for Agent {
    async fn start(&self) {
        let url_sender = self.url_sender.clone();
        let shutdown_flag = self.shutdown_flag.clone();
        tokio::spawn(async move {
            Self::dispatch_urls(url_sender, shutdown_flag).await;
        });

        // Spawn task to receive results from workers and process them
        let url_sender = self.url_sender.clone();
        let result_receiver = self.result_receiver.clone();
        tokio::spawn(async move {
            Self::process_results(url_sender, result_receiver).await;
        });
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Endpoint, QueryResult, Request};
    use chrono::Utc;
    use reqwest::StatusCode;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio::time::Duration;

    /// Helper function to set up test environment
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint to send to the channel
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// * The retry receiver channel
    /// * The process task handle
    async fn setup_test_environment(
        endpoint: Endpoint,
    ) -> (mpsc::Receiver<Endpoint>, tokio::task::JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel::<Endpoint>(10);
        let receiver = Arc::new(Mutex::new(receiver));

        sender.send(endpoint.clone()).await.unwrap();

        let (retry_sender, retry_receiver) = mpsc::channel::<Endpoint>(10);

        let process_task = tokio::spawn(async move {
            Agent::process_results(retry_sender, receiver.clone()).await;
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        (retry_receiver, process_task)
    }

    /// Helper function to create a test endpoint
    ///
    /// # Arguments
    ///
    /// * `url` - The URL for the endpoint
    /// * `statuses` - A slice of status codes for the results
    ///
    /// # Returns
    ///
    /// A configured Endpoint for testing
    fn create_test_endpoint(url: &str, statuses: &[StatusCode]) -> Endpoint {
        let results = statuses
            .iter()
            .map(|&status| QueryResult {
                status,
                timestamp: Utc::now(),
                duration: Duration::from_millis(50),
            })
            .collect();

        Endpoint {
            request: Request {
                url: url.to_string(),
                retry_delay: Duration::from_millis(100),
                retry_count: 3,
                timeout: Duration::from_secs(5),
            },
            result: results,
        }
    }

    #[tokio::test]
    async fn test_process_results_success() {
        // Create a successful Endpoint
        let successful_endpoint = create_test_endpoint("https://example.com", &[StatusCode::OK]);

        // Set up test environment
        let (mut retry_receiver, process_task) = setup_test_environment(successful_endpoint).await;

        // Abort the task
        process_task.abort();

        // Check that there are no messages in the retry channel (successful requests should not be retried)
        assert!(retry_receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_process_results_failure_with_retry() {
        // Create a failed Endpoint that hasn't reached the retry limit
        let failed_endpoint = create_test_endpoint("", &[StatusCode::INTERNAL_SERVER_ERROR]);

        // Set up test environment
        let (mut retry_receiver, process_task) =
            setup_test_environment(failed_endpoint.clone()).await;

        // Check that there is one message in the retry channel
        let retried_endpoint = retry_receiver.recv().await.unwrap();
        assert_eq!(retried_endpoint.request.url, failed_endpoint.request.url);
        assert_eq!(retried_endpoint.result.len(), 1);

        // Abort the task
        process_task.abort();
    }

    #[tokio::test]
    async fn test_process_results_failure_max_retries() {
        // Create a failed Endpoint that has reached the retry limit
        let failed_endpoint = create_test_endpoint(
            "",
            &[
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR,
            ],
        );

        // Set up test environment
        let (mut retry_receiver, process_task) = setup_test_environment(failed_endpoint).await;

        // Check that there are no messages in the retry channel (max retries reached)
        assert!(retry_receiver.try_recv().is_err());

        // Abort the task
        process_task.abort();
    }

    #[tokio::test]
    async fn test_process_results_failure_then_success() {
        // Create a custom endpoint with mixed results (first failure, then success)
        let mixed_endpoint =
            create_test_endpoint("", &[StatusCode::INTERNAL_SERVER_ERROR, StatusCode::OK]);

        // Set up test environment
        let (mut retry_receiver, process_task) = setup_test_environment(mixed_endpoint).await;

        // Check that there are no messages in the retry channel (success after retry)
        assert!(retry_receiver.try_recv().is_err());

        // Abort the task
        process_task.abort();
    }
}
