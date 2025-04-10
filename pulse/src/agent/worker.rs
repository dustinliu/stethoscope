/// Worker module for the Pulse URL monitoring system
///
/// This module implements the Worker component which is responsible for:
/// 1. Processing URL monitoring requests by making HTTP requests
/// 2. Handling various HTTP response scenarios (success, error, timeout)
/// 3. Reporting results back to the controller
use crate::message::{Endpoint, EndpointResponse, QueryResult};
use crate::task::Runnable;
use async_trait::async_trait;
use log::warn;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

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
    url_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_sender: mpsc::Sender<EndpointResponse>,
    client: reqwest::Client,
}

impl Worker {
    /// Creates a new Worker instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this worker
    /// * `url_receiver` - Receiver for incoming URL queries
    /// * `result_sender` - Sender to transmit processed query results
    ///
    /// # Returns
    /// A new Worker instance with the specified configuration
    pub fn new(
        id: usize,
        url_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
        result_sender: mpsc::Sender<EndpointResponse>,
    ) -> Self {
        Self {
            name: format!("{}-{}", WORKER_NAME_PREFIX, id),
            url_receiver,
            result_sender,
            client: reqwest::Client::new(),
        }
    }

    /// Receives a query from the channel
    ///
    /// # Returns
    /// An Option containing the received Endpoint if available
    async fn receive_endpoint(&self) -> Option<Endpoint> {
        let mut receiver = self.url_receiver.lock().await;
        receiver.recv().await
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
    async fn process_endpoint(&self, endpoint: Endpoint) {
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
                status: response.status(),
                timestamp,
                duration: start_time.elapsed(),
            },
            Err(e) => {
                if e.is_timeout() {
                    QueryResult {
                        status: reqwest::StatusCode::REQUEST_TIMEOUT,
                        timestamp,
                        duration: start_time.elapsed(),
                    }
                } else if e.is_status() {
                    QueryResult {
                        status: e.status().unwrap(),
                        timestamp,
                        duration: start_time.elapsed(),
                    }
                } else {
                    QueryResult {
                        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        timestamp,
                        duration: start_time.elapsed(),
                    }
                }
            }
        };

        let response = EndpointResponse {
            request: endpoint,
            results: vec![result],
        };

        if let Err(e) = self.result_sender.send(response).await {
            warn!("Failed to send result: {}", e);
        }
    }
}

#[async_trait]
impl Runnable for Worker {
    /// Starts the worker's endpoint processing loop
    async fn start(&self) {
        while let Some(endpoint) = self.receive_endpoint().await {
            self.process_endpoint(endpoint).await;
        }
    }

    /// Returns the name of this worker instance
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Endpoint, EndpointResponse};
    use bytes::Bytes;
    use http_body_util::Empty;
    use hyper::service::service_fn;
    use hyper::{Response as HyperResponse, StatusCode};
    use hyper_util::rt::TokioExecutor;
    use hyper_util::rt::TokioIo;
    use hyper_util::server::conn::auto::Builder;
    use pretty_assertions::assert_eq;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::thread::sleep;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    /// Creates a test endpoint with default configuration
    ///
    /// # Returns
    /// An Endpoint instance configured for testing
    fn create_test_endpoint() -> Endpoint {
        Endpoint {
            id: 0,
            url: "http://127.0.0.1:0".to_string(),
            timeout: Duration::from_secs(5),
            retry_delay: Duration::from_secs(1),
            retry_count: 3,
        }
    }

    /// Runs a worker test with a given mock server configuration
    ///
    /// # Arguments
    /// * `response_fn` - Function that generates the mock server response
    /// * `timeout` - Duration to wait for the response
    ///
    /// # Returns
    /// A tuple containing the processed endpoint and the server address
    async fn run_worker_test<F>(response_fn: F, timeout: Duration) -> (EndpointResponse, SocketAddr)
    where
        F: Fn() -> HyperResponse<Empty<Bytes>> + Clone + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let response_fn = response_fn.clone();

                let service = service_fn(move |_req| {
                    let response_fn = response_fn.clone();
                    async move { Ok::<_, Infallible>(response_fn()) }
                });

                let builder = Builder::new(TokioExecutor::new());
                if let Err(e) = builder
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                {
                    warn!("Error serving connection: {}", e);
                }
            }
        });

        let (url_sender, url_receiver) = mpsc::channel(100);
        let (result_sender, mut result_receiver) = mpsc::channel(100);

        let worker = Worker::new(0, Arc::new(Mutex::new(url_receiver)), result_sender);

        tokio::spawn(async move {
            worker.start().await;
        });

        let endpoint = Endpoint {
            id: 0,
            url: format!("http://{}", addr),
            retry_delay: Duration::from_secs(1),
            timeout,
            retry_count: 1,
        };

        url_sender.send(endpoint).await.unwrap();

        let endpoint = result_receiver.recv().await.unwrap();
        drop(url_sender);

        (endpoint, addr)
    }

    /// Tests the worker's handling of successful responses
    #[tokio::test]
    async fn test_worker_normal() {
        let (endpoint, _addr) = run_worker_test(
            || {
                HyperResponse::builder()
                    .status(StatusCode::OK)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(endpoint.results.len(), 1);
        assert_eq!(endpoint.results[0].status, reqwest::StatusCode::OK);
        assert!(endpoint.results[0].duration.as_secs_f64() > 0.0);
    }

    /// Tests the worker's handling of server errors
    #[tokio::test]
    async fn test_worker_server_error() {
        let (endpoint, _addr) = run_worker_test(
            || {
                HyperResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(endpoint.results.len(), 1);
        assert_eq!(
            endpoint.results[0].status,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(endpoint.results[0].duration.as_secs_f64() > 0.0);
    }

    #[tokio::test]
    async fn test_worker_timeout_error() {
        let (endpoint, _addr) = run_worker_test(
            || {
                sleep(Duration::from_millis(50));
                HyperResponse::builder()
                    .status(StatusCode::OK)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_millis(10),
        )
        .await;

        println!("Endpoint: {:?}", endpoint);
        assert_eq!(endpoint.results.len(), 1, "Should have exactly one result");
        assert_eq!(
            endpoint.results[0].status,
            reqwest::StatusCode::REQUEST_TIMEOUT,
            "Status should be REQUEST_TIMEOUT (408), got {}",
            endpoint.results[0].status
        );
        assert!(
            endpoint.results[0].duration.as_secs_f64() > 0.0,
            "Duration should be greater than 0"
        );
    }

    #[tokio::test]
    async fn test_worker_connection_error() {
        // Override the URL to point to a non-existent server
        let mut endpoint = create_test_endpoint();
        endpoint.url = "http://non-existent-server-123456789.com".to_string();

        let (url_sender, url_receiver) = mpsc::channel(100);
        let (result_sender, mut result_receiver) = mpsc::channel(100);
        let worker = Worker::new(0, Arc::new(Mutex::new(url_receiver)), result_sender);

        tokio::spawn(async move {
            worker.start().await;
        });

        url_sender.send(endpoint).await.unwrap();
        let endpoint = result_receiver.recv().await.unwrap();
        drop(url_sender);

        assert_eq!(endpoint.results.len(), 1);
        assert_eq!(
            endpoint.results[0].status,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(endpoint.results[0].duration.as_secs_f64() > 0.0);
    }

    #[tokio::test]
    async fn test_worker_processes_endpoint() {
        let (url_sender, url_receiver) = mpsc::channel(10);
        let (result_sender, mut result_receiver) = mpsc::channel(10);

        let worker = Worker::new(0, Arc::new(Mutex::new(url_receiver)), result_sender);

        let test_endpoint = create_test_endpoint();
        url_sender.send(test_endpoint.clone()).await.unwrap();

        let worker_handle = tokio::spawn(async move {
            worker.start().await;
        });

        let result = result_receiver.recv().await.unwrap();
        worker_handle.abort();

        assert_eq!(result.request.url, test_endpoint.url);
        assert_eq!(result.results.len(), 1);
    }
}
