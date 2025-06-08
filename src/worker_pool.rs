use crate::{
    Config,
    message::{Endpoint, QueryRecord, QueryResult},
    reporters::{Reporter, create_enabled_reporters},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{
    Mutex,
    mpsc::{self, Receiver},
};

const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

pub struct WorkerPool {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    handles: Vec<HandleHolder>,
    conf: Config,
}

impl WorkerPool {
    pub fn new(conf: Config) -> Self {
        let (endpoint_tx, endpoint_rx) =
            mpsc::channel::<Endpoint>(Self::calculate_channel_capacity(conf.worker.num_instance));
        Self {
            endpoint_tx,
            endpoint_rx: Arc::new(Mutex::new(endpoint_rx)),
            handles: Vec::new(),
            conf,
        }
    }

    pub fn run(&mut self) {
        // Create shared reporters
        let reporters: Vec<Box<dyn Reporter>> = create_enabled_reporters(&self.conf);
        let reporters = Arc::new(reporters);

        // Spawn worker tasks
        for i in 0..self.conf.worker.num_instance {
            let worker = Worker::new(i, reporters.clone(), self.endpoint_rx.clone());
            let name = worker.name.clone();

            let handle = tokio::spawn(async move {
                worker.run().await;
            });
            self.handles.push(HandleHolder {
                name: name,
                handle: handle,
            });
        }

        tracing::info!("WorkerPool started with {} workers", self.conf.worker.num_instance);
    }

    pub async fn submit(&self, endpoint: Endpoint) -> Result<(), mpsc::error::SendError<Endpoint>> {
        self.endpoint_tx.send(endpoint.clone()).await
    }

    pub async fn shutdown(self) {
        tracing::info!("Shutting down WorkerPool with {} workers", self.conf.worker.num_instance);
        drop(self.endpoint_tx);

        for handle in self.handles {
            //TODO: read config for timeout
            match tokio::time::timeout(WORKER_SHUTDOWN_TIMEOUT, handle.handle).await {
                Ok(_) => {
                    tracing::debug!("{} completed successfully", handle.name);
                }
                Err(_) => {
                    tracing::warn!("{} did not shutdonw timeout, forcing termination", handle.name);
                }
            }
        }
        tracing::info!("WorkerPool shutdown complete");
    }

    fn calculate_channel_capacity(worker_count: usize) -> usize {
        std::cmp::min(worker_count / 3, 100)
    }
}

struct HandleHolder {
    name: String,
    handle: tokio::task::JoinHandle<()>,
}

struct Worker {
    name: String,
    client: reqwest::Client,
    reporters: Arc<Vec<Box<dyn Reporter>>>,
    rx: Arc<Mutex<Receiver<Endpoint>>>,
}

impl Worker {
    fn new(
        id: usize,
        reporters: Arc<Vec<Box<dyn Reporter>>>,
        rx: Arc<Mutex<Receiver<Endpoint>>>,
    ) -> Self {
        let client = reqwest::ClientBuilder::new()
            .pool_max_idle_per_host(0)
            .pool_idle_timeout(Duration::from_secs(0))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            name: format!("Worker-{}", id),
            client,
            reporters,
            rx,
        }
    }

    async fn run(&self) {
        tracing::debug!("{} started", self.name);

        while let Some(endpoint) = self.receive().await {
            self.process_endpoint(endpoint).await;
        }

        tracing::debug!("{} stopped", self.name);
    }

    async fn receive(&self) -> Option<Endpoint> {
        let mut rx = self.rx.lock().await;
        rx.recv().await
    }

    /// Processes a single endpoint.
    ///
    /// Makes HTTP request, records result, and calls all reporters.
    async fn process_endpoint(&self, endpoint: Endpoint) {
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
                let status = if e.is_connect() {
                    reqwest::StatusCode::SERVICE_UNAVAILABLE
                } else if e.is_timeout() {
                    reqwest::StatusCode::REQUEST_TIMEOUT
                } else if e.is_status() {
                    e.status()
                        .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
                } else {
                    reqwest::StatusCode::INTERNAL_SERVER_ERROR
                };

                QueryResult {
                    endpoint,
                    record: QueryRecord {
                        status,
                        timestamp,
                        duration: start_time.elapsed(),
                    },
                }
            }
        };

        self.call_reporters(result).await;
    }

    /// Calls all configured reporters with the query result.
    ///
    /// Reporters are called independently and in parallel.
    /// Failures in one reporter don't affect others.
    async fn call_reporters(&self, result: QueryResult) {
        // Call all reporters in parallel using futures::join_all
        let reporter_futures: Vec<_> = self
            .reporters
            .iter()
            .map(|reporter| {
                let result_clone = result.clone();
                let reporter_name = reporter.name();
                async move {
                    if let Err(e) = reporter.report(result_clone).await {
                        tracing::warn!("Reporter '{}' failed: {}", reporter_name, e);
                    }
                }
            })
            .collect();

        futures::future::join_all(reporter_futures).await;
    }
}
