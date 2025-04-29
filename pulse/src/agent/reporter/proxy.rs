use std::sync::{Arc, Weak};

use anyhow::{Result, bail};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{Broker, broker::ResultReceiver, config, runnable::Runnable};

use super::{Reporter, stdout::Stdout};

pub struct Proxy {
    name: String,
    result_rx: ResultReceiver,
    reporters: Vec<Arc<Mutex<dyn Reporter>>>,
}

impl Proxy {
    pub fn new(id: usize, broker: Weak<Broker>) -> Result<Self> {
        if let Some(broker) = broker.upgrade() {
            let config = config::instance();
            let mut reporters = Vec::new();
            if config.reporter.enable_stdout {
                reporters.push(Arc::new(Mutex::new(Stdout::new(Arc::downgrade(&broker)))));
            }
            Ok(Self {
                name: format!("Report Proxy-{id}"),
                result_rx: broker.result_receiver(),
                reporters: Vec::new(),
            })
        } else {
            bail!("Broker not alive for Proxy reporter")
        }
    }
}

#[async_trait]
impl Runnable for Proxy {
    async fn run(&mut self) {
        while let Some(result) = self.result_rx.receive().await {
            let mut handles = Vec::new();
            for reporter in &mut self.reporters {
                let result = result.clone();
                let reporter = reporter.clone();
                handles.push(tokio::spawn(async move {
                    if let Err(e) = reporter.lock().await.report(result).await {
                        tracing::warn!("Failed to report result: {}", e);
                    }
                }));
            }

            for handle in handles {
                if let Err(e) = handle.await {
                    tracing::warn!("Failed to join reporter future: {}", e);
                }
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}
