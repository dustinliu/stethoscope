use crate::{
    config,
    message::{Endpoint, EndpointHistory, QueryResult},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{fmt, sync::Arc};
use tokio::sync::{Mutex, broadcast, mpsc, watch};

// The Broker struct acts as a central message hub for different components
// of the application (controller, agents, reporter). It facilitates communication
// between these components using asynchronous channels.
#[derive(Debug)]
pub struct Broker {
    // Channel for sending endpoints to be monitored by agents.
    endpoint_tx: mpsc::Sender<Endpoint>,
    // Channel for receiving endpoints (shared access for multiple agents).
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    // Channel for sending query results from agents.
    result_tx: mpsc::Sender<QueryResult>,
    // Channel for receiving query results (shared access for the controller).
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    // Broadcast channel for sending endpoint history reports to reporters.
    report_tx: broadcast::Sender<EndpointHistory>,
    // Broadcast channel receiver for reporters to get reports.
    report_rx: broadcast::Receiver<EndpointHistory>,

    // Watch channel for broadcasting the shutdown signal.
    shutdown_tx: watch::Sender<bool>,
    // Watch channel receiver to listen for the shutdown signal.
    shutdown_rx: watch::Receiver<bool>,
}

impl Broker {
    // Creates a new Broker instance, initializing all the necessary channels.
    // Channel buffer sizes and other configurations are taken from the global config.
    pub fn new() -> Self {
        let config = config::instance();
        let (endpoint_tx, endpoint_rx) = mpsc::channel(100);
        let (result_tx, result_rx) = mpsc::channel(100);
        let (report_tx, report_rx) = broadcast::channel(config.reporter.alert_buffer_len);

        let endpoint_rx = Arc::new(Mutex::new(endpoint_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            endpoint_tx,
            endpoint_rx,
            result_tx,
            result_rx,
            report_tx,
            report_rx,
            shutdown_tx,
            shutdown_rx,
        }
    }

    // Internal helper method to send an item through an mpsc channel,
    // incorporating a check for the shutdown signal.
    // Returns an error if the channel is closed or if a shutdown is initiated.
    async fn _send_with_shutdown<T>(
        &self,
        sender: &mpsc::Sender<T>,
        item: T,
        operation_desc: &str, // Description of the operation for error messages.
    ) -> Result<()>
    where
        T: Send + Sync + fmt::Debug + 'static,
    {
        let item_desc = format!("{:?}", item);
        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::select! {
            result = sender.send(item) => {
                result.with_context(|| format!("failed to send {}: {}", operation_desc, item_desc))?
            },
            _ = shutdown_rx.changed() => {
                bail!("broker is shutting down, cannot send {}: {}", operation_desc, item_desc);
            }
        }
        Ok(())
    }

    // Internal helper method to receive an item from a mutex-protected mpsc receiver,
    // incorporating a check for the shutdown signal.
    // Returns None if the channel is closed or if a shutdown is initiated.
    async fn _receive_with_shutdown_option<T>(
        &self,
        receiver_mutex: &Arc<Mutex<mpsc::Receiver<T>>>,
    ) -> Option<T>
    where
        T: Send + 'static,
    {
        let mut shutdown_rx = self.shutdown_rx.clone();
        let mut guard = receiver_mutex.lock().await;
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                None
            }
            result = guard.recv() => {
                result
            }
        }
    }

    // Sends an endpoint to the agents for monitoring.
    // Uses the internal helper `_send_with_shutdown` for safety.
    pub async fn send_endpoint(&self, endpoint: Endpoint) -> Result<()> {
        self._send_with_shutdown(&self.endpoint_tx, endpoint, "endpoint")
            .await
    }

    // Sends a query result from an agent to the controller.
    // Uses the internal helper `_send_with_shutdown` for safety.
    pub async fn send_result(&self, result: QueryResult) -> Result<()> {
        self._send_with_shutdown(&self.result_tx, result, "query result")
            .await
    }

    // Sends an endpoint history report via the broadcast channel.
    // Returns the number of active receivers that received the report, or an error.
    pub fn send_report(&self, history: EndpointHistory) -> Result<usize> {
        self.report_tx
            .send(history)
            .with_context(|| "failed to send report")
    }

    // Receives an endpoint from the channel for an agent to process.
    // Returns None if the channel is closed or shutdown is initiated.
    pub async fn receive_endpoint(&self) -> Option<Endpoint> {
        self._receive_with_shutdown_option(&self.endpoint_rx).await
    }

    // Receives a query result from the channel for the controller to process.
    // Returns None if the channel is closed or shutdown is initiated.
    pub async fn receive_result(&self) -> Option<QueryResult> {
        self._receive_with_shutdown_option(&self.result_rx).await
    }

    // Receives an endpoint history report from the broadcast channel.
    // Returns an error if the channel lags or if shutdown is initiated.
    pub async fn receive_report(&mut self) -> Result<EndpointHistory> {
        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                Err(anyhow!("broker is shutting down, cannot receive report"))
            }
            result = self.report_rx.recv() => {
                result.map_err(anyhow::Error::from)
            }
        }
    }

    // Initiates the shutdown process by sending a signal on the watch channel.
    // Panics if sending the signal fails (which should generally not happen).
    pub fn shutdown(&self) {
        self.shutdown_tx
            .send(true)
            .expect("failed to send shutdown signal");
    }

    // Waits asynchronously until the shutdown signal is received.
    // Panics if waiting on the watch channel fails.
    pub async fn wait_for_shutdown(&mut self) {
        self.shutdown_rx
            .changed()
            .await
            .expect("failed to wait for shutdown signal");
    }
}

// Implements the Clone trait for the Broker.
// Cloning creates new handles to the existing channels, allowing multiple
// components to interact with the same Broker instance.
// Note that the report_rx is subscribed, creating a new independent receiver.
impl Clone for Broker {
    fn clone(&self) -> Self {
        Self {
            endpoint_tx: self.endpoint_tx.clone(),
            endpoint_rx: self.endpoint_rx.clone(),
            result_tx: self.result_tx.clone(),
            result_rx: self.result_rx.clone(),
            report_tx: self.report_tx.clone(),
            report_rx: self.report_tx.subscribe(),
            shutdown_tx: self.shutdown_tx.clone(),
            shutdown_rx: self.shutdown_tx.subscribe(),
        }
    }
}

struct EnddpointSender {
    endpoint_tx: mpsc::Sender<Endpoint>,
}

impl EnddpointSender {
    fn new(endpoint_tx: &mpsc::Sender<Endpoint>) -> Self {
        Self {
            endpoint_tx: endpoint_tx.clone(),
        }
    }

    async fn send(&self, endpoint: Endpoint) -> Result<()> {
        self.endpoint_tx
            .send(endpoint)
            .await
            .with_context(|| "failed to send endpoint")
    }
}

struct EndpointReceiver {
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
}

impl EndpointReceiver {
    fn new(endpoint_rx: &Arc<Mutex<mpsc::Receiver<Endpoint>>>) -> Self {
        Self {
            endpoint_rx: endpoint_rx.clone(),
        }
    }

    async fn receive(&self) -> Option<Endpoint> {
        self.endpoint_rx.lock().await.recv().await
    }
}

struct ResultSender {
    result_tx: mpsc::Sender<QueryResult>,
}

impl ResultSender {
    fn new(result_tx: &mpsc::Sender<QueryResult>) -> Self {
        Self {
            result_tx: result_tx.clone(),
        }
    }

    async fn send(&self, result: QueryResult) -> Result<()> {
        self.result_tx
            .send(result)
            .await
            .with_context(|| "failed to send QueryResult")
    }
}

struct ResultReceiver {
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
}

impl ResultReceiver {
    fn new(result_rx: &Arc<Mutex<mpsc::Receiver<QueryResult>>>) -> Self {
        Self {
            result_rx: result_rx.clone(),
        }
    }

    async fn receive(&self) -> Option<QueryResult> {
        self.result_rx.lock().await.recv().await
    }
}

struct ReportSender {
    report_tx: broadcast::Sender<EndpointHistory>,
}

impl ReportSender {
    fn new(report_tx: &broadcast::Sender<EndpointHistory>) -> Self {
        Self {
            report_tx: report_tx.clone(),
        }
    }

    async fn send(&self, history: EndpointHistory) -> Result<usize> {
        self.report_tx
            .send(history)
            .with_context(|| "failed to send report")
    }
}

struct ReportReceiver {
    report_rx: broadcast::Receiver<EndpointHistory>,
}

impl ReportReceiver {
    fn new(report_tx: &broadcast::Sender<EndpointHistory>) -> Self {
        Self {
            report_rx: report_tx.subscribe(),
        }
    }

    async fn receive(&mut self) -> Result<EndpointHistory> {
        self.report_rx
            .recv()
            .await
            .with_context(|| "failed to receive report")
    }
}
