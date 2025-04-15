use crate::message::{Endpoint, EndpointHistory, QueryResult};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, mpsc};

pub struct Broker {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_tx: mpsc::Sender<QueryResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    status_tx: broadcast::Sender<EndpointHistory>,
    status_rx: broadcast::Receiver<EndpointHistory>,
}

impl Broker {
    pub fn new() -> Self {
        let (endpoint_tx, endpoint_rx) = mpsc::channel(100);
        let (result_tx, result_rx) = mpsc::channel(100);
        let (status_tx, status_rx) = broadcast::channel(100);

        let endpoint_rx = Arc::new(Mutex::new(endpoint_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));

        Self {
            endpoint_tx,
            endpoint_rx,
            result_tx,
            result_rx,
            status_tx,
            status_rx,
        }
    }

    pub async fn send_endpoint(
        &self,
        endpoint: Endpoint,
    ) -> Result<(), mpsc::error::SendError<Endpoint>> {
        self.endpoint_tx.send(endpoint).await
    }

    pub async fn send_result(
        &self,
        result: QueryResult,
    ) -> Result<(), mpsc::error::SendError<QueryResult>> {
        self.result_tx.send(result).await
    }

    pub async fn send_status(
        &self,
        history: EndpointHistory,
    ) -> Result<usize, broadcast::error::SendError<EndpointHistory>> {
        self.status_tx.send(history)
    }

    pub async fn receive_endpoint(&self) -> Option<Endpoint> {
        self.endpoint_rx.lock().await.recv().await
    }

    pub async fn receive_result(&self) -> Option<QueryResult> {
        self.result_rx.lock().await.recv().await
    }

    pub async fn receive_status(&mut self) -> Result<EndpointHistory, broadcast::error::RecvError> {
        self.status_rx.recv().await
    }
}

impl Clone for Broker {
    fn clone(&self) -> Self {
        Self {
            endpoint_tx: self.endpoint_tx.clone(),
            endpoint_rx: self.endpoint_rx.clone(),
            result_tx: self.result_tx.clone(),
            result_rx: self.result_rx.clone(),
            status_tx: self.status_tx.clone(),
            status_rx: self.status_tx.subscribe(),
        }
    }
}
