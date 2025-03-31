/// Worker module for the Pulse URL monitoring system
///
/// This module implements the Worker component which is responsible for:
/// 1. Processing URL monitoring requests by making HTTP requests
/// 2. Handling various HTTP response scenarios (success, error, timeout)
/// 3. Reporting results back to the controller
// Import required dependencies for HTTP requests, async operations, and thread-safe data structures
use crate::message::{Endpoint, QueryResult};
use crate::task::Runnable;
use async_trait::async_trait;
use log::{debug, warn};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

const WORKER_NAME_PREFIX: &str = "Worker";
/// Worker struct responsible for processing URL queries asynchronously
///
/// The Worker component handles the actual HTTP requests to monitored URLs,
/// processes responses, and reports results back to the controller.
///
/// # Fields
/// * `url_receiver` - Receiver for incoming URL queries, wrapped in Arc<Mutex> for thread-safe access
/// * `result_sender` - Sender to transmit processed query results back to the controller
/// * `client` - Shared HTTP client instance for making requests
/// * `shutdown_flag` - Flag to indicate if the worker should shut down
pub struct Worker {
    name: String,
    // Receiver for incoming URL queries, wrapped in Arc<Mutex> for thread-safe access
    // Arc provides shared ownership across threads, Mutex ensures thread-safe access
    url_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    // Sender to transmit processed query results back to the controller
    result_sender: mpsc::Sender<Endpoint>,
    // Shared HTTP client instance
    client: reqwest::Client,
}

impl Worker {
    // Constructor for creating a new Worker instance
    // Takes a thread-safe receiver for queries and a sender for results
    pub fn new(
        id: usize,
        url_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
        result_sender: mpsc::Sender<Endpoint>,
    ) -> Self {
        Self {
            name: format!("{}-{}", WORKER_NAME_PREFIX, id),
            url_receiver,
            result_sender,
            client: reqwest::Client::new(),
        }
    }

    // Receive a query from the channel
    async fn receive_endpoint(&self) -> Option<Endpoint> {
        let mut receiver = self.url_receiver.lock().await;
        receiver.recv().await
    }

    // Process a single endpoint and send the result back
    async fn process_endpoint(&self, mut endpoint: Endpoint) {
        // Execute endpoint and store result
        let timestamp = chrono::Utc::now();
        let start_time = tokio::time::Instant::now();

        let result = match self
            .client
            .get(&endpoint.request.url)
            .timeout(endpoint.request.timeout)
            .send()
            .await
        {
            Ok(response) => QueryResult {
                status: response.status(),
                timestamp,
                duration: start_time.elapsed(),
            },
            Err(e) => {
                // Check if the error is a timeout error
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

        endpoint.result.push(result);

        // Send the result back
        if let Err(e) = self.result_sender.send(endpoint).await {
            warn!("Failed to send result: {}", e);
        }
    }
}

#[async_trait]
impl Runnable for Worker {
    async fn start(&self) {
        while let Some(endpoint) = self.receive_endpoint().await {
            self.process_endpoint(endpoint).await;
        }
        debug!("Worker detected channel closed");
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Endpoint, Request};
    use bytes::Bytes;
    use http_body_util::Empty;
    use hyper::service::service_fn;
    use hyper::{Response, StatusCode};
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

    // Helper function to create a test endpoint
    fn create_test_endpoint() -> Endpoint {
        Endpoint {
            request: Request {
                url: "http://127.0.0.1:0".to_string(),
                timeout: Duration::from_secs(5),
                retry_delay: Duration::from_secs(1),
                retry_count: 3,
            },
            result: Vec::new(),
        }
    }

    // Helper function to run a worker test with a given mock server configuration
    async fn run_worker_test<F>(response_fn: F, timeout: Duration) -> (Endpoint, SocketAddr)
    where
        F: Fn() -> Response<Empty<Bytes>> + Clone + Send + 'static,
    {
        // Create a TCP listener with a random port
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

        // Spawn the worker in a separate task
        tokio::spawn(async move {
            worker.start().await;
        });

        let endpoint = Endpoint {
            request: Request {
                url: format!("http://{}", addr),
                retry_delay: Duration::from_secs(1),
                timeout,
                retry_count: 1,
            },
            result: vec![],
        };

        url_sender.send(endpoint).await.unwrap();

        let endpoint = result_receiver.recv().await.unwrap();
        drop(url_sender);

        (endpoint, addr)
    }

    #[tokio::test]
    async fn test_worker_normal() {
        let (endpoint, _addr) = run_worker_test(
            || {
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(endpoint.result.len(), 1);
        assert_eq!(endpoint.result[0].status, reqwest::StatusCode::OK);
        assert!(endpoint.result[0].duration.as_secs_f64() > 0.0);
    }

    #[tokio::test]
    async fn test_worker_server_error() {
        let (endpoint, _addr) = run_worker_test(
            || {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(endpoint.result.len(), 1);
        assert_eq!(
            endpoint.result[0].status,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(endpoint.result[0].duration.as_secs_f64() > 0.0);
    }

    #[tokio::test]
    async fn test_worker_timeout_error() {
        let (endpoint, _addr) = run_worker_test(
            || {
                sleep(Duration::from_millis(50));
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Empty::new())
                    .unwrap()
            },
            Duration::from_millis(10),
        )
        .await;

        println!("Endpoint: {:?}", endpoint);
        assert_eq!(endpoint.result.len(), 1, "Should have exactly one result");
        assert_eq!(
            endpoint.result[0].status,
            reqwest::StatusCode::REQUEST_TIMEOUT,
            "Status should be REQUEST_TIMEOUT (408), got {}",
            endpoint.result[0].status
        );
        assert!(
            endpoint.result[0].duration.as_secs_f64() > 0.0,
            "Duration should be greater than 0"
        );
    }

    #[tokio::test]
    async fn test_worker_connection_error() {
        // Override the URL to point to a non-existent server
        let mut endpoint = create_test_endpoint();
        endpoint.request.url = "http://non-existent-server-123456789.com".to_string();
        endpoint.result = vec![];

        let (url_sender, url_receiver) = mpsc::channel(100);
        let (result_sender, mut result_receiver) = mpsc::channel(100);
        let worker = Worker::new(0, Arc::new(Mutex::new(url_receiver)), result_sender);

        tokio::spawn(async move {
            worker.start().await;
        });

        url_sender.send(endpoint).await.unwrap();
        let endpoint = result_receiver.recv().await.unwrap();
        drop(url_sender);

        assert_eq!(endpoint.result.len(), 1);
        assert_eq!(
            endpoint.result[0].status,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(endpoint.result[0].duration.as_secs_f64() > 0.0);
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

        assert_eq!(result.request.url, test_endpoint.request.url);
        assert_eq!(result.result.len(), 1);
    }
}
