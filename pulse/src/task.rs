/// Task module for the Pulse URL monitoring system
///
/// This module defines the Runnable trait which is implemented by all
/// components that need to run asynchronously in the system.
use async_trait::async_trait;

/// Trait for components that can be run asynchronously
///
/// This trait is implemented by all major components (Dispatcher, Worker, Reporter)
/// that need to run continuously in the background.
///
/// # Methods
/// * `start` - Begins the asynchronous execution of the component
/// * `name` - Returns the name identifier of the component
#[async_trait]
pub trait Runnable {
    /// Starts the asynchronous execution of the component
    async fn start(&self);

    /// Returns the name identifier of the component
    fn name(&self) -> &str;
}
