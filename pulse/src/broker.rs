use crate::{
    config,
    error::BrokerError,
    message::{Endpoint, EndpointHistory, QueryResult},
};
use anyhow::{Context, Result, anyhow};
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

    pub fn register_endpoint_sender(&self) -> EndpointSender {
        EndpointSender::new(self.endpoint_tx.clone(), self.shutdown_tx.subscribe())
    }

    pub fn register_result_sender(&self) -> ResultSender {
        ResultSender::new(self.result_tx.clone(), self.shutdown_tx.subscribe())
    }

    pub fn register_report_sender(&self) -> ReportSender {
        ReportSender::new(self.report_tx.clone(), self.shutdown_tx.subscribe())
    }

    pub fn register_endpoint_receiver(&self) -> EndpointReceiver {
        EndpointReceiver::new(self.endpoint_rx.clone(), self.shutdown_tx.subscribe())
    }

    pub fn register_result_receiver(&self) -> ResultReceiver {
        ResultReceiver::new(self.result_rx.clone(), self.shutdown_tx.subscribe())
    }

    pub fn register_report_receiver(&self) -> ReportReceiver {
        ReportReceiver::new(self.report_tx.subscribe(), self.shutdown_tx.subscribe())
    }

    pub fn register_shutdown_sender(&self) -> ShutdownSender {
        ShutdownSender::new(self.shutdown_tx.clone())
    }

    pub fn register_shutdown_receiver(&self) -> ShutdownReceiver {
        ShutdownReceiver::new(self.shutdown_tx.subscribe())
    }

    pub fn shutdown(self) {
        self.shutdown_tx.send(true);
        drop(self.endpoint_tx);
        drop(self.endpoint_rx);
        drop(self.result_tx);
        drop(self.result_rx);
        drop(self.report_tx);
        drop(self.report_rx);
        drop(self.shutdown_tx);
        drop(self.shutdown_rx);
    }
}

#[derive(Clone)]
struct MpscSender<T> {
    tx: mpsc::Sender<T>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<T> MpscSender<T>
where
    T: Send + fmt::Debug + Sync + 'static,
{
    fn new(tx: mpsc::Sender<T>, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self { tx, shutdown_rx }
    }

    pub async fn send(&mut self, value: T) -> Result<()> {
        let value_desc = format!("{:?}", value);
        self.tx
            .send(value)
            .await
            .with_context(|| format!("failed to send value: {:?}", value_desc))
    }

    pub async fn is_shutdown(&mut self) -> Result<()> {
        self.shutdown_rx
            .changed()
            .await
            .with_context(|| "failed to wait for shutdown")
    }
}

// Generic wrapper for MPSC Receiver
#[derive(Clone)]
struct MpscReceiver<T> {
    rx: Arc<Mutex<mpsc::Receiver<T>>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<T> MpscReceiver<T>
where
    T: Send + 'static,
{
    fn new(rx: Arc<Mutex<mpsc::Receiver<T>>>, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self { rx, shutdown_rx }
    }

    pub async fn receive(&self) -> Option<T> {
        let mut guard = self.rx.lock().await;
        guard.recv().await
    }

    pub async fn is_shutdown(&mut self) -> Result<()> {
        self.shutdown_rx
            .changed()
            .await
            .with_context(|| "failed to wait for shutdown")
    }
}

// Generic wrapper for Broadcast Sender
#[derive(Clone)]
struct BroadcastSender<T> {
    tx: broadcast::Sender<T>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<T> BroadcastSender<T>
where
    T: Clone + Send + fmt::Debug + Sync + 'static,
{
    fn new(tx: broadcast::Sender<T>, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self { tx, shutdown_rx }
    }

    // Send is synchronous for broadcast channels
    pub fn send(&self, value: T) -> Result<usize> {
        let value_desc = format!("{:?}", value);
        self.tx
            .send(value)
            // Reverting to with_context as requested by user
            .with_context(|| format!("failed to send broadcast value: {}", value_desc))
    }

    pub async fn is_shutdown(&mut self) -> Result<()> {
        self.shutdown_rx
            .changed()
            .await
            .with_context(|| "failed to wait for shutdown")
    }
}

// Generic wrapper for Broadcast Receiver
// Does not derive Clone as broadcast::Receiver is not Clone
pub struct BroadcastReceiver<T> {
    rx: broadcast::Receiver<T>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<T> BroadcastReceiver<T>
where
    T: Clone + Send + Sync + 'static,
{
    // Create a new receiver by subscribing to the sender
    fn new(rx: broadcast::Receiver<T>, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self { rx, shutdown_rx }
    }

    pub async fn receive(&mut self) -> Result<T> {
        self.rx
            .recv()
            .await
            .with_context(|| format!("receive broadcast value error"))
    }
}

pub struct ShutdownSender {
    tx: watch::Sender<bool>,
}

impl ShutdownSender {
    pub fn new(tx: watch::Sender<bool>) -> Self {
        Self { tx }
    }

    pub fn send(&self) -> Result<()> {
        self.tx.send(true);
        Ok(())
    }
}

pub struct ShutdownReceiver {
    rx: watch::Receiver<bool>,
}

impl ShutdownReceiver {
    pub fn new(rx: watch::Receiver<bool>) -> Self {
        Self { rx }
    }

    pub async fn wait(&mut self) -> Result<()> {
        self.rx.changed().await?;
        Ok(())
    }
}

// --- Type Aliases for Specific Channel Wrappers ---

pub type EndpointSender = MpscSender<Endpoint>;
pub type EndpointReceiver = MpscReceiver<Endpoint>;
pub type ResultSender = MpscSender<QueryResult>;
pub type ResultReceiver = MpscReceiver<QueryResult>;
pub type ReportSender = BroadcastSender<EndpointHistory>;
pub type ReportReceiver = BroadcastReceiver<EndpointHistory>;
