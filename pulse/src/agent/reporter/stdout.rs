extern crate self as pulse;

use async_trait::async_trait;
use pulse_proc_macros::reporter;
use tokio::io::{self, AsyncWriteExt};

use crate::{broker::Broker, message::EndpointHistory};

use super::Executor;

#[reporter]
pub struct Stdout {
    name: String,
    broker: Broker,
    stdout: io::Stdout,
}

impl Stdout {
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("stdout-{}", id),
            broker,
            stdout: io::stdout(),
        }
    }
}

#[async_trait]
impl Executor for Stdout {
    async fn report(&mut self, report: EndpointHistory) {
        let message = format!("{}", report);
        let _ = self.stdout.write_all(message.as_bytes()).await;
        let _ = self.stdout.flush().await;
    }
}
