/// Worker module for the Pulse URL monitoring system
///
/// This module implements the Worker component which is responsible for:
/// 1. Processing URL monitoring requests by making HTTP requests
/// 2. Handling various HTTP response scenarios (success, error, timeout)
/// 3. Reporting results back to the controller
use crate::runnable::Runnable;
use crate::{
    broker::Broker,
    message::{QueryRecord, QueryResult},
};
use async_trait::async_trait;
use log::warn;

/// Prefix for worker instance names
const WORKER_NAME_PREFIX: &str = "Worker";

/// Worker responsible for processing URL queries asynchronously
///
/// The Worker component handles the actual HTTP requests to monitored URLs,
/// processes responses, and reports results back to the controller.
///
/// # Fields
/// * `name` - Name of the worker instance
/// * `url_receiver` - Receiver for incoming URL queries
/// * `result_sender` - Sender to transmit processed query results
/// * `client` - Shared HTTP client instance for making requests
pub struct Worker {
    name: String,
    broker: Broker,
    client: reqwest::Client,
}

impl Worker {
    /// Creates a new Worker instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this worker
    /// * `broker` - Broker instance to use for communication
    ///
    /// # Returns
    /// A new Worker instance with the specified configuration
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", WORKER_NAME_PREFIX, id),
            broker,
            client: reqwest::Client::new(),
        }
    }

    /// Process a single endpoint and send the result back
    ///
    /// This method:
    /// 1. Makes an HTTP request to the endpoint URL
    /// 2. Records the response status and timing
    /// 3. Sends the result back through the result channel
    ///
    /// # Arguments
    /// * `endpoint` - The endpoint to process
    async fn process_endpoint(&self) {
        if let Some(endpoint) = self.broker.receive_endpoint().await {
            let timestamp = chrono::Utc::now();
            let start_time = tokio::time::Instant::now();

            let result = match self
                .client
                .get(&endpoint.url)
                .timeout(endpoint.timeout)
                .send()
                .await
            {
                Ok(response) => QueryResult {
                    endpoint,
                    record: QueryRecord {
                        status: response.status(),
                        timestamp,
                        duration: start_time.elapsed(),
                    },
                },
                Err(e) => {
                    if e.is_connect() {
                        QueryResult {
                            endpoint,
                            record: QueryRecord {
                                status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
                                timestamp,
                                duration: start_time.elapsed(),
                            },
                        }
                    } else if e.is_timeout() {
                        QueryResult {
                            endpoint,
                            record: QueryRecord {
                                status: reqwest::StatusCode::REQUEST_TIMEOUT,
                                timestamp,
                                duration: start_time.elapsed(),
                            },
                        }
                    } else if e.is_status() {
                        QueryResult {
                            endpoint,
                            record: QueryRecord {
                                status: e.status().unwrap(),
                                timestamp,
                                duration: start_time.elapsed(),
                            },
                        }
                    } else {
                        QueryResult {
                            endpoint,
                            record: QueryRecord {
                                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                                timestamp,
                                duration: start_time.elapsed(),
                            },
                        }
                    }
                }
            };

            if let Err(e) = self.broker.send_result(result).await {
                warn!("Failed to send result: {}", e);
            }
        }
    }
}

#[async_trait]
impl Runnable for Worker {
    /// Starts the worker's endpoint processing loop
    async fn run(&self) {
        self.process_endpoint().await;
    }

    /// Returns the name of this worker instance
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Endpoint, QueryResult};
    use httptest::{
        Expectation, Server,
        matchers::*,
        responders::{self},
    };
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Runs a worker test with a given mock server configuration
    ///
    /// # Arguments
    /// * `status_code` - HTTP status code to return from the mock server
    /// * `delay` - Optional delay before sending the response
    /// * `timeout` - Duration to wait for the response
    ///
    /// # Returns
    /// A tuple containing the processed query result and the server address
    async fn run_worker_test(
        status_code: reqwest::StatusCode,
        delay: Option<Duration>,
        timeout: Duration,
    ) -> QueryResult {
        // Start a server running on a local ephemeral port
        let server = Server::run();
        let addr = server.addr();

        // Configure the server to expect a GET request and respond with the specified status code
        let expectation = if let Some(delay) = delay {
            Expectation::matching(request::method_path("GET", "/")).respond_with(
                responders::delay_and_then(delay, responders::status_code(status_code.as_u16())),
            )
        } else {
            Expectation::matching(request::method_path("GET", "/"))
                .respond_with(responders::status_code(status_code.as_u16()))
        };

        server.expect(expectation);

        let broker = Broker::new();
        let worker = Worker::new(0, broker.clone());

        let worker_handle = tokio::spawn(async move {
            worker.run().await;
        });

        let endpoint = Endpoint {
            id: 0,
            url: format!("http://{}", addr),
            timeout,
            failure_threshold: 3,
        };

        broker.send_endpoint(endpoint).await.unwrap();
        let result = broker.receive_result().await.unwrap();

        worker_handle.abort();

        println!("Result: {:?}", result);
        result
    }

    /// Tests the worker's handling of successful responses
    #[tokio::test]
    async fn test_worker_normal() {
        let result = run_worker_test(reqwest::StatusCode::OK, None, Duration::from_secs(5)).await;

        println!("Result: {:?}", result);
        assert_eq!(result.record.status, reqwest::StatusCode::OK);
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }

    /// Tests the worker's handling of server errors
    #[tokio::test]
    async fn test_worker_server_error() {
        let result = run_worker_test(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            None,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(
            result.record.status,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }

    #[tokio::test]
    async fn test_worker_timeout_error() {
        let result = run_worker_test(
            reqwest::StatusCode::OK,
            Some(Duration::from_millis(50)),
            Duration::from_millis(10),
        )
        .await;

        assert_eq!(result.record.status, reqwest::StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test]
    async fn test_worker_connection_error() {
        // Override the URL to point to a non-existent server
        let endpoint = Endpoint {
            id: 0,
            url: "http://jklfjkfjk".to_string(),
            timeout: Duration::from_secs(5),
            failure_threshold: 3,
        };

        let broker = Broker::new();
        let worker = Worker::new(0, broker.clone());

        tokio::spawn(async move {
            worker.run().await;
        });

        broker.send_endpoint(endpoint).await.unwrap();
        let result = broker.receive_result().await.unwrap();

        assert_eq!(
            result.record.status,
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }
}
