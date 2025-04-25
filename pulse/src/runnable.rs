use crate::broker::Broker;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Weak;

/// Trait for components that can be run asynchronously
///
/// This trait is implemented by all major components (Dispatcher, Worker, Reporter)
/// that need to run continuously in the background.
///
/// # Methods
/// * `start` - Begins the asynchronous execution of the component
/// * `name` - Returns the name identifier of the component
#[async_trait]
pub trait Runnable: Sized {
    fn new(id: usize, broker: Weak<Broker>) -> Result<Self>;

    /// Starts the asynchronous execution of the component
    async fn run(&mut self);

    /// Returns the name identifier of the component
    fn name(&self) -> &str;
}
