pub mod proxy;
mod stdout;

pub use proxy::Proxy;

use crate::message::QueryResult;
use anyhow::Result;
use async_trait::async_trait;

// Defines the interface for different reporting mechanisms.
// Reporters implement this trait to process and deliver endpoint history reports.
#[async_trait]
trait Reporter: Send + Sync {
    async fn report(&mut self, result: QueryResult) -> Result<()>;
}

// stdio reporter module
