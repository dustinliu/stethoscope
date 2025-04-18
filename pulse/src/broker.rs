use crate::message::{Endpoint, EndpointHistory, QueryResult};
use log::error;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, mpsc, watch};

pub struct Broker {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_tx: mpsc::Sender<QueryResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    report: broadcast::Sender<EndpointHistory>,
    report_rx: broadcast::Receiver<EndpointHistory>,

    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl Broker {
    pub fn new() -> Self {
        let (endpoint_tx, endpoint_rx) = mpsc::channel(100);
        let (result_tx, result_rx) = mpsc::channel(100);
        let (report_tx, report_rx) = broadcast::channel(100);

        let endpoint_rx = Arc::new(Mutex::new(endpoint_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            endpoint_tx,
            endpoint_rx,
            result_tx,
            result_rx,
            report: report_tx,
            report_rx,
            shutdown_tx,
            shutdown_rx,
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

    pub async fn send_report(
        &self,
        history: EndpointHistory,
    ) -> Result<usize, broadcast::error::SendError<EndpointHistory>> {
        self.report.send(history)
    }

    pub async fn receive_endpoint(&self) -> Option<Endpoint> {
        self.endpoint_rx.lock().await.recv().await
    }

    pub async fn receive_result(&self) -> Option<QueryResult> {
        self.result_rx.lock().await.recv().await
    }

    pub async fn receive_report(&mut self) -> Result<EndpointHistory, broadcast::error::RecvError> {
        self.report_rx.recv().await
    }

    pub fn shutdown(&self) {
        self.shutdown_tx
            .send(true)
            .expect("failed to send shutdown signal");
    }

    pub async fn wait_for_shutdown(&mut self) {
        self.shutdown_rx
            .changed()
            .await
            .expect("failed to wait for shutdown signal");
    }

    pub fn is_shutdown(&self) -> bool {
        if let Ok(changed) = self.shutdown_rx.has_changed() {
            changed
        } else {
            error!("failed to check if shutdown signal has changed, shutting down");
            true
        }
    }
}

impl Clone for Broker {
    fn clone(&self) -> Self {
        Self {
            endpoint_tx: self.endpoint_tx.clone(),
            endpoint_rx: self.endpoint_rx.clone(),
            result_tx: self.result_tx.clone(),
            result_rx: self.result_rx.clone(),
            report: self.report.clone(),
            report_rx: self.report.subscribe(),
            shutdown_tx: self.shutdown_tx.clone(),
            shutdown_rx: self.shutdown_tx.subscribe(),
        }
    }
}
