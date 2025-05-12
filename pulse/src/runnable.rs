use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::{sync::Mutex, task::JoinHandle, time::timeout};

/// Trait for components that can be run asynchronously
///
/// This trait is implemented by all major components (Dispatcher, Worker, Reporter)
/// that need to run continuously in the background.
///
/// # Methods
/// * `start` - Begins the asynchronous execution of the component
/// * `name` - Returns the name identifier of the component
#[async_trait]
pub trait Runnable: Send + Sync {
    /// Starts the asynchronous execution of the component
    async fn run(&mut self);

    /// Returns the name identifier of the component
    fn name(&self) -> &str;
}

pub struct TasksGroup {
    name: String,
    tasks: Vec<Arc<Mutex<Box<dyn Runnable>>>>,
    handles: Vec<JoinHandle<()>>,
}

impl TasksGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tasks: vec![],
            handles: vec![],
        }
    }

    pub fn add_task(&mut self, task: Box<dyn Runnable>) {
        self.tasks.push(Arc::new(Mutex::new(task)));
    }

    pub async fn wait_for_shutdown(&mut self, handle_timeout: Duration, group_timeout: Duration) {
        tracing::debug!("{} waiting for shutdown", self.name);
        match timeout(group_timeout, self.shutdown(handle_timeout)).await {
            Ok(()) => {
                tracing::info!("{} shutdown complete", self.name);
            }
            Err(_) => {
                tracing::warn!(
                    "{} shutdown timed out after {} seconds",
                    self.name,
                    group_timeout.as_secs()
                );
            }
        }
    }

    pub async fn shutdown(&mut self, wait_timeout: Duration) {
        for (i, handle) in self.handles.iter_mut().enumerate() {
            let task = self.tasks[i].lock().await;
            tracing::debug!("{} waiting for {} seconds", task.name(), wait_timeout.as_secs());
            match timeout(wait_timeout, &mut *handle).await {
                Ok(result) => match result {
                    Ok(()) => {
                        tracing::debug!("{} shutdown complete", task.name());
                    }

                    Err(_) => {
                        let task = self.tasks[i].lock().await;
                        tracing::warn!("failed to wait for shutdown of {}", task.name());
                    }
                },
                Err(_) => {
                    tracing::warn!(
                        "{} shutdown timed out after {} seconds",
                        task.name(),
                        wait_timeout.as_secs()
                    );
                    handle.abort();
                }
            }
        }
    }

    pub fn run(&mut self) {
        for task in self.tasks.iter_mut() {
            let task = task.clone();
            let handle = tokio::spawn(async move {
                let mut task = task.lock().await;
                task.run().await;
            });
            self.handles.push(handle);
        }
    }
}
