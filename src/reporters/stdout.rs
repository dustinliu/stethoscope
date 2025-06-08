use super::Reporter;
use crate::message::QueryResult;
use anyhow::Result;
use async_trait::async_trait;

/// Reporter that outputs results to stdout.
///
/// This reporter formats and prints query results to the standard output
/// in a human-readable format showing timestamp, URL, status, and duration.
pub struct StdoutReporter;

impl StdoutReporter {
    /// Creates a new StdoutReporter instance.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Reporter for StdoutReporter {
    async fn report(&self, result: QueryResult) -> Result<()> {
        println!(
            "[{}] {} - {} ({}ms)",
            result.record.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            result.endpoint.url,
            result.record.status,
            result.record.duration.as_millis()
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "stdout"
    }
}
