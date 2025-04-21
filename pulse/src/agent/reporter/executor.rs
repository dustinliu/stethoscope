use async_trait::async_trait;

use crate::message::EndpointHistory;

// Defines the interface for different reporting mechanisms.
// Reporters implement this trait to process and deliver endpoint history reports.
#[async_trait]
pub trait Executor: Send + Sync {
    // Processes and reports the given endpoint history.
    // This method is called by the reporting infrastructure when an alert needs to be sent.
    async fn report(&mut self, history: EndpointHistory);
}

// stdio reporter module
