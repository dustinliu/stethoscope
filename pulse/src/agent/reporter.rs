use std::sync::Arc;

use super::report_executor::{Executor, StdIO};
use crate::{broker::Broker, config, runnable::Runnable};
use async_trait::async_trait;

pub struct Reporter {
    name: String,
    broker: Broker,
    executors: Vec<Option<Arc<dyn Executor>>>,
}

impl Reporter {
    pub fn new(id: usize, broker: Broker) -> Self {
        let mut executors = Vec::new();
        let config = config::instance();
        if config.reporter.enable_stdout {
            executors.push(Some(Arc::new(StdIO::new()) as Arc<dyn Executor>));
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
        while let Some(report) = self.broker.receive_report().await {
            let mut handles = vec![];

            for executor in self.executors.iter().flatten() {
                let e = executor.clone();
                let r = report.clone();
                handles.push(tokio::spawn(async move { e.report(r.clone()).await }));
            }

            for handle in handles {
                handle.await.unwrap();
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
