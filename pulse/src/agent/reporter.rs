use super::report_executor::{Executor, StdIO};
use crate::{broker::Broker, config, runnable::Runnable};
use async_trait::async_trait;
use log::{error, warn};
use tokio::sync::broadcast;

// stdio reporter module

pub struct Reporter {
    name: String,
    broker: Broker,
    executors: Vec<Option<Box<dyn Executor>>>,
}

impl Reporter {
    pub fn new(id: usize, broker: Broker) -> Self {
        let mut executors = Vec::new();
        let config = config::instance();
        if config.enable_stdio_reporter() {
            executors.push(Some(Box::new(StdIO::new()) as Box<dyn Executor>));
        }

        Self {
            name: format!("StdIO Reporter {}", id),
            broker,
            executors,
        }
    }
}

#[async_trait]
impl Runnable for Reporter {
    async fn run(&mut self) {
        loop {
            match self.broker.receive_report().await {
                Ok(report) => {
                    for executor in self.executors.iter().flatten() {
                        executor.report(report.clone()).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("alert buffer is lagged by {} messages", n);
                }
                Err(e) => {
                    error!("failed to receive report: {}", e);
                    break;
                }
            }

            if self.broker.is_shutdown() {
                break;
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}
