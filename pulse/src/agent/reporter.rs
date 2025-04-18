use super::report_executor::{Executor, StdIO};
use crate::{broker::Broker, config, runnable::Runnable};
use async_trait::async_trait;

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
        while let Ok(report) = self.broker.receive_report().await {
            for executor in self.executors.iter().flatten() {
                executor.report(report.clone()).await;
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
