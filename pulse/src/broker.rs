use crate::{
    config::Config,
    message::{Endpoint, QueryResult},
};
use anyhow::{Context, Result};
use std::{
    fmt,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

#[derive(Debug)]
struct MpscHolder<T> {
    state: Mutex<MpscState<T>>,
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<T>>>,
}

#[derive(Debug)]
struct MpscState<T> {
    tx: Option<mpsc::Sender<T>>,
    weak_tx: Option<mpsc::WeakSender<T>>,
}

impl<T> MpscHolder<T> {
    fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            state: Mutex::new(MpscState {
                tx: Some(tx),
                weak_tx: None,
            }),
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }

    fn receiver(&self) -> Arc<tokio::sync::Mutex<mpsc::Receiver<T>>> {
        self.rx.clone()
    }

    fn sender(&self) -> mpsc::Sender<T> {
        let mut state = self.state.lock().expect("Failed to lock MpscState");

        if let Some(weak_tx) = &state.weak_tx {
            if let Some(tx) = weak_tx.upgrade() {
                return tx;
            }
        }

        if let Some(tx) = state.tx.take() {
            state.weak_tx = Some(tx.downgrade());
            return tx;
        }

        //this should never happen
        panic!("sender already taken");
    }
}

// The Broker struct acts as a central message hub for different components
// of the application (controller, agents, reporter). It facilitates communication
// between these components using asynchronous channels.
#[derive(Debug)]
pub struct Broker {
    endpoint_holder: MpscHolder<Endpoint>,
    result_holder: MpscHolder<QueryResult>,
}

impl Broker {
    // Creates a new Broker instance, initializing all the necessary channels.
    // Channel buffer sizes are derived from the provided worker configuration.
    pub fn new(config: &Config) -> Self {
        Self {
            endpoint_holder: MpscHolder::new(config.worker.num_instance),
            result_holder: MpscHolder::new(config.worker.num_instance),
        }
    }

    pub fn endpoint_sender(&self) -> EndpointSender {
        EndpointSender::new(self.endpoint_holder.sender())
    }

    pub fn result_sender(&self) -> ResultSender {
        ResultSender::new(self.result_holder.sender())
    }

    pub fn endpoint_receiver(&self) -> EndpointReceiver {
        EndpointReceiver::new(self.endpoint_holder.receiver())
    }

    pub fn result_receiver(&self) -> ResultReceiver {
        ResultReceiver::new(self.result_holder.receiver())
    }

    pub fn shutdown(self) {
        // MpscHolder 會在 drop 時自動清理資源
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
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<T>>>,
}

impl<T> MpscReceiver<T>
where
    T: Send + 'static,
{
    fn new(rx: Arc<tokio::sync::Mutex<mpsc::Receiver<T>>>) -> Self {
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
