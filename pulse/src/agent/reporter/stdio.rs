// stdio reporter module

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::broker::Broker;
use crate::runnable::Runnable;

pub struct StdIO {
    name: String,
    broker: Mutex<Broker>,
}

impl StdIO {
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("StdIO Reporter {}", id),
            broker: Mutex::new(broker),
        }
    }
}

#[async_trait]
impl Runnable for StdIO {
    async fn run(&self) {
        let report = self.broker.lock().await.receive_report().await;
        if let Ok(report) = report {
            println!("{}", report);
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}
