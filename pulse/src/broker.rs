use crate::{
    config,
    message::{Endpoint, EndpointHistory, QueryResult},
};
use anyhow::{Context, Result, anyhow};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, mpsc, watch};

#[derive(Debug)]
pub struct Broker {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_tx: mpsc::Sender<QueryResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    report_tx: broadcast::Sender<EndpointHistory>,
    report_rx: broadcast::Receiver<EndpointHistory>,

    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl Broker {
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

    pub async fn send_endpoint(&self, endpoint: Endpoint) -> Result<()> {
        let endpoint_desc = format!("{:?}", endpoint);
        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::select! {
            result = self.endpoint_tx.send(endpoint) => {
                result.with_context(|| format!("failed to send endpoint: {}", endpoint_desc))?
            },
            _ = shutdown_rx.changed() => {
                anyhow::bail!("broker is shutting down, cannot send endpoint: {}", endpoint_desc);
            }
        }
        Ok(())
    }

    pub async fn send_result(&self, result: QueryResult) -> Result<()> {
        let result_desc = format!("{:?}", result);
        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::select! {
            send_result = self.result_tx.send(result) => {
                send_result.with_context(|| format!("failed to send query result: {}", result_desc))?
            },
            _ = shutdown_rx.changed() => {
                anyhow::bail!("broker is shutting down, cannot send query result: {}", result_desc);
            }
        }
        Ok(())
    }

    pub fn send_report(&self, history: EndpointHistory) -> Result<usize> {
        self.report_tx
            .send(history)
            .with_context(|| "failed to send report")
    }

    pub async fn receive_endpoint(&self) -> Option<Endpoint> {
        let mut shutdown_rx = self.shutdown_rx.clone();
        let mut guard = self.endpoint_rx.lock().await;
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

    pub async fn receive_result(&self) -> Option<QueryResult> {
        let mut shutdown_rx = self.shutdown_rx.clone();
        let mut guard = self.result_rx.lock().await;
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
            tracing::error!("failed to check if shutdown signal has changed, shutting down");
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
            report_tx: self.report_tx.clone(),
            report_rx: self.report_tx.subscribe(),
            shutdown_tx: self.shutdown_tx.clone(),
            shutdown_rx: self.shutdown_tx.subscribe(),
        }
    }
}
