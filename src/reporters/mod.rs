mod file;
mod stdout;

use crate::{Config, message::QueryResult};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

pub use file::FileReporter;
pub use stdout::StdoutReporter;

/// Trait for different reporting mechanisms.
///
/// Reporters implement this trait to process and deliver endpoint monitoring results.
/// Each reporter should handle errors independently and not affect other reporters.
#[async_trait]
pub trait Reporter: Send + Sync {
    async fn report(&self, result: QueryResult) -> Result<()>;

    /// Returns the name of this reporter for logging purposes.
    fn name(&self) -> &'static str;
}

pub fn create_enabled_reporters(conf: &Config) -> Vec<Box<dyn Reporter>> {
    let mut reporters: Vec<Box<dyn Reporter>> = Vec::new();

    if conf.reporter.enable_stdout {
        reporters.push(Box::new(StdoutReporter::new()));
    }

    if let Some(ref file_path) = conf.reporter.file_path {
        reporters.push(Box::new(FileReporter::new(PathBuf::from(file_path))));
    }

    // Future reporters can be added here:
    // if config.reporter.enable_file {
    //     reporters.push(Box::new(FileReporter::new()));
    // }
    // if config.reporter.enable_webhook {
    //     reporters.push(Box::new(WebhookReporter::new()));
    // }

    reporters
}
