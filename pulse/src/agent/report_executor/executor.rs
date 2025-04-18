use async_trait::async_trait;

use crate::message::EndpointHistory;

#[async_trait]
pub trait Executor: Send + Sync {
    async fn report(&self, history: EndpointHistory);
}

// stdio report_executor module
