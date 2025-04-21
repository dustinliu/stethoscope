extern crate self as pulse;

use async_trait::async_trait;
use pulse_proc_macros::reporter;
use tokio::io::{self, AsyncWriteExt};

use crate::{broker::Broker, message::EndpointHistory};

use super::Executor;

// A reporter that prints endpoint history reports to standard output.
// The `#[reporter]` attribute likely generates the boilerplate `Runnable` implementation.
#[reporter]
pub struct Stdout {
    name: String,
    broker: Broker,
    stdout: io::Stdout, // Handle to standard output.
}

impl Stdout {
    // Creates a new Stdout reporter instance.
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
    // Writes the formatted report to standard output asynchronously.
    async fn report(&mut self, report: EndpointHistory) {
        let message = format!("{}", report);
        let _ = self.stdout.write_all(message.as_bytes()).await;
        let _ = self.stdout.flush().await;
    }
}
