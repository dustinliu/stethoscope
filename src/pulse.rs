use crate::{Config, EndpointProvider, WorkerPool, message::Endpoint};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::watch,
    task::JoinHandle,
    time,
};

const SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(10);

pub struct Pulse {
    conf: Config,
    endpoints: Vec<Endpoint>,
    shutdown_flag: Arc<AtomicBool>,
}

impl Pulse {
    pub fn new(conf: Config) -> Self {
        Self {
            conf,
            endpoints: EndpointProvider::new().get_endpoints(),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn start(&self) {
        tracing::info!("Pulse started with {} endpoints", self.endpoints.len());
        let (tx, rx) = watch::channel::<bool>(false);
        let handle = self.process_endpoints(rx).await;

        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        loop {
            tokio::select! {
                _ = sigint_stream.recv() => {
                    tracing::info!("SIGINT received, shutdown initiated...");
                    tx.send(true).expect("Failed to send shutdown signal");
                    self.shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                _ = sigterm_stream.recv() => {
                    tracing::info!("SIGTERM received, shutdown initiated...");
                    tx.send(true).expect("Failed to send shutdown signal");
                    self.shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
            }
        }

        let _ = handle.await;
    }

    async fn process_endpoints(&self, mut rx: watch::Receiver<bool>) -> JoinHandle<()> {
        let conf = self.conf.clone();
        let endpoints = self.endpoints.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        tokio::spawn(async move {
            let mut interval = time::interval(conf.check_interval);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

            let mut worker_pool = WorkerPool::new(conf);
            worker_pool.run();

            'outter: loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::debug!("Processing endpoints...");
                        for endpoint in &endpoints {
                            if let Err(e) = worker_pool.submit(endpoint.clone()).await {
                                tracing::error!("Failed to process endpoint {}: {}", endpoint.id, e);
                            }
                            if shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                tracing::debug!("Shutdown flag set, stopping endpoint processing");
                                break 'outter;
                            }
                        }
                        tracing::debug!("Endpoint processing cycle completed, waiting for next tick");
                    },
                    _ = rx.changed() => {
                        tracing::debug!("Shutdown signal received, stopping endpoint processing");
                        break 'outter;
                    }
                }
            }

            let shutdown_timeout = tokio::time::timeout(SHUTDOWN_TIMEOUT_SECS, async {
                worker_pool.shutdown().await;
            });

            if let Err(e) = shutdown_timeout.await {
                tracing::warn!("workers shutdown timeout reached: {}, force quit", e);
            }
        })
    }
}
