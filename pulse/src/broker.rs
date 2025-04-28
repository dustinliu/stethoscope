use crate::{
    config,
    message::{Endpoint, QueryResult},
};
use anyhow::{Context, Result};
use std::{fmt, sync::Arc};
use tokio::sync::{Mutex, mpsc};

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
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    // Creates a new Broker instance, initializing all the necessary channels.
    // Channel buffer sizes and other configurations are taken from the global config.
    pub fn new() -> Self {
        let worker_num = config::instance().worker.num_instance;
        let (endpoint_tx, endpoint_rx) = mpsc::channel(worker_num / 3);
        let (result_tx, result_rx) = mpsc::channel(worker_num);

        let endpoint_rx = Arc::new(Mutex::new(endpoint_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));

        Self {
            endpoint_tx,
            endpoint_rx,
            result_tx,
            result_rx,
        }
    }

    pub fn endpoint_sender(&self) -> EndpointSender {
        EndpointSender::new(self.endpoint_tx.clone())
    }

    pub fn result_sender(&self) -> ResultSender {
        ResultSender::new(self.result_tx.clone())
    }

    pub fn endpoint_receiver(&self) -> EndpointReceiver {
        EndpointReceiver::new(self.endpoint_rx.clone())
    }

    pub fn result_receiver(&self) -> ResultReceiver {
        ResultReceiver::new(self.result_rx.clone())
    }

    pub fn shutdown(self) {
        drop(self.endpoint_tx);
        drop(self.endpoint_rx);
        drop(self.result_tx);
        drop(self.result_rx);
    }
}

#[derive(Clone)]
pub struct MpscSender<T> {
    tx: mpsc::Sender<T>,
}

impl<T> MpscSender<T>
where
    T: Send + fmt::Debug + Sync + 'static,
{
    fn new(tx: mpsc::Sender<T>) -> Self {
        Self { tx }
    }

    pub async fn send(&mut self, value: T) -> Result<()> {
        let value_desc = format!("{:?}", value);
        self.tx
            .send(value)
            .await
            .with_context(|| format!("failed to send value: {:?}", value_desc))
    }
}

// Generic wrapper for MPSC Receiver
#[derive(Clone)]
pub struct MpscReceiver<T> {
    rx: Arc<Mutex<mpsc::Receiver<T>>>,
}

impl<T> MpscReceiver<T>
where
    T: Send + 'static,
{
    fn new(rx: Arc<Mutex<mpsc::Receiver<T>>>) -> Self {
        Self { rx }
    }

    pub async fn receive(&self) -> Option<T> {
        let mut guard = self.rx.lock().await;
        guard.recv().await
    }
}

// --- Type Aliases for Specific Channel Wrappers ---

pub type EndpointSender = MpscSender<Endpoint>;
pub type EndpointReceiver = MpscReceiver<Endpoint>;
pub type ResultSender = MpscSender<QueryResult>;
pub type ResultReceiver = MpscReceiver<QueryResult>;
