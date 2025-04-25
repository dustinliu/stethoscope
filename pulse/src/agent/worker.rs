use crate::{
    broker::{Broker, EndpointReceiver, ResultSender},
    message::{Endpoint, QueryRecord, QueryResult},
    runnable::Runnable,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Weak;

/// Prefix for worker instance names
const WORKER_NAME_PREFIX: &str = "Worker";

/// Worker responsible for processing URL queries asynchronously.
///
/// Receives `Endpoint`s from the `Broker`, performs HTTP GET requests,
/// handles potential errors (connection, timeout, status codes), and sends
/// `QueryResult`s back to the `Broker`.
///
/// # Fields
/// * `name` - Name of the worker instance (e.g., "Worker-0").
/// * `broker` - Cloned `Broker` instance for communication.
/// * `client` - `reqwest::Client` used for making HTTP requests.
pub struct Worker {
    name: String,
    endpoint_rx: EndpointReceiver,
    result_tx: ResultSender,
    client: reqwest::Client,
}

impl Worker {
    /// The main loop for the worker.
    /// Continuously receives `Endpoint`s from the broker's endpoint channel.
    /// For each endpoint, it performs an HTTP GET request, records the result
    /// (status, timestamp, duration), handles errors appropriately (mapping
    /// reqwest errors to specific status codes like `SERVICE_UNAVAILABLE` or
    /// `REQUEST_TIMEOUT`), and sends the `QueryResult` back via the broker's
    /// result channel. The loop terminates when the broker's endpoint channel
    /// is closed or a shutdown signal is received.
    async fn process_endpoint(&mut self, endpoint: Endpoint) {
        let timestamp = chrono::Utc::now();
        let start_time = tokio::time::Instant::now();

        let result = match self
            .client
            .get(&endpoint.url)
            .timeout(endpoint.timeout)
            .header(reqwest::header::CONNECTION, "close")
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

        if let Err(e) = self.result_tx.send(result).await {
            tracing::warn!("Failed to send result: {}", e);
        }
    }
}

#[async_trait]
impl Runnable for Worker {
    fn new(id: usize, broker: Weak<Broker>) -> Result<Self> {
        if let Some(broker) = broker.upgrade() {
            Ok(Self {
                name: format!("{}-{}", WORKER_NAME_PREFIX, id),
                endpoint_rx: broker.register_endpoint_receiver(),
                result_tx: broker.register_result_sender(),
                client: reqwest::ClientBuilder::new()
                    .pool_max_idle_per_host(0) // Disable keeping idle connections
                    .pool_idle_timeout(std::time::Duration::from_secs(0)) // Set idle timeout to 0
                    .build()
                    .with_context(|| "Failed to build reqwest client {}")?,
            })
        } else {
            Err(anyhow::anyhow!("Broker is not alive"))
        }
    }

    /// Starts the worker's main processing loop (`process_endpoint`).
    async fn run(&mut self) {
        loop {
            tokio::select! {
                Some(endpoint) = self.endpoint_rx.receive() => {
                    self.process_endpoint(endpoint).await;
                }
                _ = self.result_tx.is_shutdown() => {
                    tracing::info!("{}: Shutdown signal received", self.name);
                    break;
                }
                else => {
                    tracing::info!("{}: Endpoint channel closed, shutting down.", self.name);
                    break;
                }
            }
        }
    }

    /// Returns the name of this worker instance.
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
    use std::{sync::Arc, time::Duration};

    /// Runs a worker test with a given mock server configuration.
    /// Sets up a mock HTTP server (`httptest::Server`) to simulate responses.
    /// Creates a `Worker` and runs it in a separate task.
    /// Sends a test `Endpoint` to the worker via the `Broker`.
    /// Receives the `QueryResult` from the worker via the `Broker`.
    ///
    /// # Arguments
    /// * `status_code` - HTTP status code for the mock server to return.
    /// * `delay` - Optional delay before the mock server sends the response.
    /// * `timeout` - Request timeout duration for the `Endpoint`.
    ///
    /// # Returns
    /// The `QueryResult` produced by the worker.
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

        let broker = Arc::new(Broker::new());
        let mut worker =
            Worker::new(0, Arc::downgrade(&broker)).expect("Failed to create worker for test");

        let worker_handle = tokio::spawn(async move {
            worker.run().await;
        });

        let endpoint = Endpoint {
            id: 0,
            url: format!("http://{}", addr),
            timeout,
            failure_threshold: 3,
        };

        broker
            .register_endpoint_sender()
            .send(endpoint)
            .await
            .unwrap();
        let result = broker.register_result_receiver().receive().await.unwrap();

        worker_handle.abort();

        println!("Result: {:?}", result);
        result
    }

    /// Tests the worker's handling of successful (200 OK) responses.
    #[tokio::test]
    async fn test_worker_normal() {
        let result = run_worker_test(reqwest::StatusCode::OK, None, Duration::from_secs(5)).await;

        println!("Result: {:?}", result);
        assert_eq!(result.record.status, reqwest::StatusCode::OK);
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }

    /// Tests the worker's handling of server-side errors (500 Internal Server Error).
    #[tokio::test]
    async fn test_worker_server_error() {
        let result = run_worker_test(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            None,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(result.record.status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }

    /// Tests the worker's handling of request timeouts.
    /// Uses a short endpoint timeout and a longer server delay.
    /// Expects the result status to be `REQUEST_TIMEOUT`.
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

    /// Tests the worker's handling of connection errors.
    /// Sends a request to a non-existent address.
    /// Expects the result status to be `SERVICE_UNAVAILABLE`.
    #[tokio::test]
    async fn test_worker_connection_error() {
        // Override the URL to point to a non-existent server
        let endpoint = Endpoint {
            id: 0,
            url: "http://jklfjkfjk".to_string(),
            timeout: Duration::from_secs(5),
            failure_threshold: 3,
        };

        let broker = Arc::new(Broker::new());
        let mut worker = Worker::new(0, Arc::downgrade(&broker))
            .expect("Failed to create worker for connection error test");

        let worker_handle = tokio::spawn(async move {
            worker.run().await;
        });

        broker
            .register_endpoint_sender()
            .send(endpoint)
            .await
            .unwrap();
        let result = broker.register_result_receiver().receive().await.unwrap();

        worker_handle.abort();

        assert_eq!(result.record.status, reqwest::StatusCode::SERVICE_UNAVAILABLE);
        assert!(result.record.duration.as_secs_f64() > 0.0);
    }
}
