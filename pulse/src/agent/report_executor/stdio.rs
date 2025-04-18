use super::Executor;
use crate::message::EndpointHistory;
use async_trait::async_trait;

pub struct StdIO;

impl StdIO {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Executor for StdIO {
    async fn report(&self, history: EndpointHistory) {
        println!("{}", history);
    }
}
