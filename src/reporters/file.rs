use super::Reporter;
use crate::message::QueryResult;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

/// Reporter that writes results to a file.
///
/// This reporter appends query results to a specified log file
/// in JSON format for structured logging and analysis.
pub struct FileReporter {
    file_path: PathBuf,
}

impl FileReporter {
    /// Creates a new FileReporter instance.
    ///
    /// # Arguments
    /// * `file_path` - Path to the log file where results will be written
    pub fn new(file_path: PathBuf) -> Self {
        Self { file_path }
    }
}

#[async_trait]
impl Reporter for FileReporter {
    async fn report(&self, result: QueryResult) -> Result<()> {
        let log_entry = serde_json::json!({
            "timestamp": result.record.timestamp.to_rfc3339(),
            "endpoint_id": result.endpoint.id,
            "url": result.endpoint.url,
            "status_code": result.record.status.as_u16(),
            "duration_ms": result.record.duration.as_millis(),
            "success": result.record.status.is_success()
        });

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .await
            .with_context(|| format!("Failed to open log file: {}", self.file_path.display()))?;

        let log_line = format!("{}\n", log_entry);
        file.write_all(log_line.as_bytes())
            .await
            .with_context(|| "Failed to write to log file")?;

        file.flush()
            .await
            .with_context(|| "Failed to flush log file")?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "file"
    }
}
