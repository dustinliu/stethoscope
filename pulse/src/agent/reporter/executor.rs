use async_trait::async_trait;

use crate::message::EndpointHistory;

#[async_trait]
pub trait Executor: Send + Sync {
    async fn report(&mut self, history: EndpointHistory);
}

// stdio reporter module
