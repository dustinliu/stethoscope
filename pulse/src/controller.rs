use crate::{
    agent::{Aggregator, Dispatcher, StdIO, Worker},
    broker::Broker,
    config::{self, Config},
    runnable::Runnable,
};
use log::{debug, info, warn};
use std::{sync::Arc, time::Duration};
use tokio::{
    signal::unix::{SignalKind, signal},
    task::JoinHandle,
    time::timeout,
};

const SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(10);
const TASK_SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(3);
const AGGREGATOR_NUM: usize = 1;
const DISPATCHER_NUM: usize = 1;

/// Controller that manages the URL monitoring system
///
/// Coordinates between agents and workers by:
/// 1. Managing channels for request distribution and response collection
/// 2. Spawning and managing agent and worker tasks
/// 3. Monitoring the health of agents and workers
///
/// # Fields
/// * `shutdown_sender` - Channel for sending shutdown signals to all tasks
pub struct Controller {
    broker: Broker,
    config: &'static Config,
}

impl Controller {
    /// Creates a new Controller with channels for request and response communication
    ///
    /// Initializes channels with a buffer size of 100 for both request and response channels
    pub fn new() -> Self {
        Self {
            broker: Broker::new(),
            config: config::instance(),
        }
    }

    /// Starts the monitoring system
    ///
    /// This method:
    /// 1. Spawns agent and worker tasks for parallel processing
    /// 2. Starts monitoring tasks to ensure agents and workers are running
    /// 3. Continuously receives and processes responses
    pub async fn start(&self) {
        let mut tasks = Vec::new();

        if self.config.enable_stdio_reporter() {
            let stdio_handle = self.run_agent(1, StdIO::new);
            tasks.push(TaskHandle {
                name: "StdIO Reporter Group".to_string(),
                handle: stdio_handle,
            });
        }

        tasks.push(TaskHandle {
            name: "Aggregator Group".to_string(),
            handle: self.run_agent(AGGREGATOR_NUM, Aggregator::new),
        });
        tasks.push(TaskHandle {
            name: "Worker Group".to_string(),
            handle: self.run_agent(self.config.worker_num(), Worker::new),
        });
        tasks.push(TaskHandle {
            name: "Dispatcher Group".to_string(),
            handle: self.run_agent(DISPATCHER_NUM, Dispatcher::new),
        });

        // Wait for SIGINT or SIGTERM to initiate shutdown
        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        tokio::select! {
            _ = sigint_stream.recv() => {
                info!("SIGINT received, shutdown initiated...");
                self.broker.shutdown();
            }
            _ = sigterm_stream.recv() => {
                info!("SIGTERM received, shutdown initiated...");
                self.broker.shutdown();
            }
        }

        for task in tasks.into_iter().rev() {
            Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, &task.name, task.handle).await;
        }

        info!("All tasks shutdown complete");
    }

    fn run_agent<T, F>(&self, num_tasks: usize, task_factory: F) -> JoinHandle<()>
    where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize, Broker) -> T,
    {
        let agents = (0..num_tasks)
            .map(|i| Arc::new(task_factory(i, self.broker.clone())))
            .collect::<Vec<_>>();

        let mut handles = Vec::with_capacity(num_tasks);

        for agent in &agents {
            let agent = agent.clone();
            let handle = tokio::spawn(async move {
                agent.run().await;
            });
            handles.push(handle);
        }

        let mut broker = self.broker.clone();
        tokio::spawn(async move {
            broker.wait_for_shutdown().await;
            for (i, handle) in handles.into_iter().enumerate() {
                Self::wait_for_shutdown(TASK_SHUTDOWN_TIMEOUT_SECS, agents[i].name(), handle).await;
            }
        })
    }

    async fn wait_for_shutdown<T>(
        wait_timeout: Duration,
        task_name: &str,
        handle: tokio::task::JoinHandle<T>,
    ) {
        debug!("waiting for {} shutdown", task_name);
        match timeout(wait_timeout, handle).await {
            Ok(result) => {
                if let Err(e) = result {
                    warn!("{} shutdown error: {}", task_name, e);
                } else {
                    debug!("{} shutdown completed successfully", task_name);
                }
            }
            Err(_) => {
                warn!(
                    "{} shutdown timed out after {} seconds",
                    task_name,
                    wait_timeout.as_secs()
                );
            }
        }
    }
}

struct TaskHandle {
    name: String,
    handle: JoinHandle<()>,
}
