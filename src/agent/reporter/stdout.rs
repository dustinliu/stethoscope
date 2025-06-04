extern crate self as pulse;

use anyhow::{Result, bail};
use async_trait::async_trait;
use std::sync::Weak;
use tokio::io::{self, AsyncWriteExt};

use crate::{
    broker::{Broker, ResultReceiver},
    message::QueryResult,
    runnable::Runnable,
};

use super::Reporter;

// A reporter that prints endpoint history reports to standard output.
// Runnable trait is implemented manually below.
pub struct Stdout {
    name: &'static str,
    result_rx: ResultReceiver,
    stdout: io::Stdout, // Handle to standard output.
}

impl Stdout {
    pub fn new(broker: Weak<Broker>) -> Result<Self> {
        if let Some(broker) = broker.upgrade() {
            Ok(Self {
                name: "stdout reporter",
                result_rx: broker.result_receiver(),
                stdout: io::stdout(),
            })
        } else {
            bail!("Broker not alive for Stdout reporter")
        }
    }
}

// Implement Runnable manually
#[async_trait]
impl Runnable for Stdout {
    async fn run(&mut self) {
        while let Some(result) = self.result_rx.receive().await {
            let message = format!("{:?}", result);
            let _ = self.stdout.write_all(message.as_bytes()).await;
            let _ = self.stdout.flush().await;
        }
    }

    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait]
impl Reporter for Stdout {
    async fn report(&mut self, result: QueryResult) -> Result<()> {
        let message = format!("{:?}", result);
        let _ = self.stdout.write_all(message.as_bytes()).await;
        let _ = self.stdout.flush().await;
        Ok(())
    }
}
