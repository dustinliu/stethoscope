extern crate self as pulse;

use anyhow::{Result, bail};
use async_trait::async_trait;
use std::sync::Weak;
use tokio::{
    io::{self, AsyncWriteExt},
    sync::broadcast::error::RecvError,
};

use crate::{
    broker::{Broker, ReportReceiver, ReportSender},
    message::EndpointHistory,
    runnable::Runnable,
};

use super::Executor;

// A reporter that prints endpoint history reports to standard output.
// Runnable trait is implemented manually below.
pub struct Stdout {
    name: String,
    report_rx: ReportReceiver,
    report_tx: ReportSender, // Keep sender for potential future shutdown mechanisms
    stdout: io::Stdout,      // Handle to standard output.
}

// Implement Runnable manually
#[async_trait]
impl Runnable for Stdout {
    fn new(id: usize, broker: Weak<Broker>) -> Result<Self> {
        if let Some(broker_arc) = broker.upgrade() {
            let report_rx = broker_arc.register_report_receiver();
            let report_tx = broker_arc.register_report_sender();
            Ok(Self {
                name: format!("stdout-{}", id),
                report_rx,
                report_tx,
                stdout: io::stdout(),
            })
        } else {
            bail!("Broker not alive for Stdout reporter")
        }
    }

    async fn run(&mut self) {
        loop {
            tokio::select! {
                // Listen for incoming reports
                // Use try_recv if we want non-blocking check alongside shutdown, but recv() waits
                // recv() returns Result<T, broadcast::error::RecvError>
                res = self.report_rx.receive() => {
                    match res {
                        Ok(report) => {
                            self.report(report).await;
                        }
                        Err(e) => {
                            // Check for specific RecvError types after downcasting
                            if let Some(RecvError::Lagged(n)) = e.downcast_ref::<RecvError>() {
                                tracing::warn!("{}: Report receiver lagged by {} messages.", self.name, *n);
                                // Continue loop after lag
                            } else if let Some(RecvError::Closed) = e.downcast_ref::<RecvError>() {
                                tracing::info!("{}: Report channel closed by sender, shutting down.", self.name);
                                break; // Exit loop on Closed
                            } else {
                                // Handle any other error (including downcast failure) as fatal
                                tracing::error!("{}: Unrecoverable error receiving report: {}. Shutting down.", self.name, e);
                                break; // Exit loop on other errors
                            }
                        }
                    }
                }

                // Check for shutdown signal using the sender's is_shutdown method
                res = self.report_tx.is_shutdown() => {
                    if let Err(e) = res {
                         tracing::error!("{}: Error waiting for shutdown signal: {}. Shutting down anyway.", self.name, e);
                    } else {
                         tracing::info!("{}: Shutdown signal received.", self.name);
                    }
                    break;
                }

                // Handle the case where the receiver channel is definitively closed
                // The receive branch now handles Closed error, so 'else' might be redundant
                // unless is_shutdown() fails in a way that doesn't break the loop.
                // However, keeping it covers potential unforeseen select! completion.
                // else => {
                //     tracing::info!("{}: Select completed unexpectedly, shutting down.", self.name);
                //     break;
                // }
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// Keep Executor implementation
#[async_trait]
impl Executor for Stdout {
    // Writes the formatted report to standard output asynchronously.
    async fn report(&mut self, report: EndpointHistory) {
        let message = format!("{}", report);
        let _ = self.stdout.write_all(message.as_bytes()).await;
        let _ = self.stdout.flush().await;
    }
}
