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
        loop {
            // Lock broker to check shutdown status
            {
                let broker_guard = self.broker.lock().await;
                if broker_guard.is_shutdown() {
                    break;
                }
                // broker_guard dropped here
            }

            // Now receive_report without holding the lock
            // Lock again just for receive_report
            let report_result = {
                let mut broker_guard = self.broker.lock().await;
                broker_guard.receive_report().await
            };

            match report_result {
                Ok(report) => {
                    println!("{}", report);
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}
