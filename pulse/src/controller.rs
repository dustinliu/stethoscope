use crate::{
    agent::reporter::Stdout,
    agent::{Aggregator, Dispatcher, Worker},
    broker::Broker,
    config::{self, Config},
    runnable::Runnable,
};
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    signal::unix::{SignalKind, signal},
    task::JoinHandle,
    time::timeout,
};

const SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(10);
const TASK_SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(3);
const AGGREGATOR_NUM: usize = 1;
const DISPATCHER_NUM: usize = 1;

/// Controller that manages the URL monitoring system lifecycle.
///
/// Responsibilities:
/// 1. Initializes and holds the central `Broker` instance.
/// 2. Spawns and manages the lifecycle of agent tasks (Dispatcher, Worker, Aggregator, Reporters).
/// 3. Listens for termination signals (SIGINT, SIGTERM) to initiate graceful shutdown.
/// 4. Coordinates the shutdown sequence of all tasks.
///
/// # Fields
/// * `broker` - The central message broker for inter-task communication.
/// * `config` - Static reference to the application configuration.
pub struct Controller {
    broker: Arc<Broker>,
    config: &'static Config,
}

impl Controller {
    /// Creates a new Controller instance.
    ///
    /// Initializes the `Broker` and loads the application configuration.
    pub fn new() -> Self {
        Self {
            broker: Some(Arc::new(Broker::new())),
            config: config::instance(),
        }
    }

    /// Starts the monitoring system and waits for termination signals.
    ///
    /// This method:
    /// 1. Spawns task groups for reporters, aggregators, workers, and dispatchers.
    /// 2. Waits for SIGINT or SIGTERM.
    /// 3. Initiates graceful shutdown via the `Broker`.
    /// 4. Waits for all spawned tasks to complete their shutdown sequence.
    pub async fn start(&mut self) {
        let mut tasks = Vec::new();
        if self.config.reporter.enable_stdout {
            self.add_task_group::<Stdout, _>(&mut tasks, "Stdout Reporter", 1, Stdout::new);
        }
        self.add_task_group::<Aggregator, _>(
            &mut tasks,
            "Aggregator",
            AGGREGATOR_NUM,
            Aggregator::new,
        );
        self.add_task_group::<Worker, _>(
            &mut tasks,
            "Worker",
            self.config.worker.num_instance,
            Worker::new,
        );
        self.add_task_group::<Dispatcher, _>(
            &mut tasks,
            "Dispatcher",
            DISPATCHER_NUM,
            Dispatcher::new,
        );

        let shutdown_tx = self.broker.register_shutdown_sender();

        // Wait for SIGINT or SIGTERM to initiate shutdown
        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        tokio::select! {
            _ = sigint_stream.recv() => {
                tracing::info!("SIGINT received, shutdown initiated...");
                if let Some(broker) = self.broker.as_ref() {
                    broker.shutdown();
                }
            }
            _ = sigterm_stream.recv() => {
                tracing::info!("SIGTERM received, shutdown initiated...");
                if let Some(broker) = self.broker.as_ref() {
                    broker.shutdown();
                }
            }
        }

        let _ = self.broker.take();

        for task in tasks.into_iter().rev() {
            tracing::info!("waiting for {} shutdown", task.name);
            Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, &task.name, task.handle).await;
            tracing::info!("{} shutdown complete", task.name);
        }

        tracing::info!("All tasks shutdown complete");
        // Drop broker after shutdown
    }

    // Spawns a group of tasks for a specific agent type (T).
    // It creates `num_tasks` instances of the agent using the `task_factory`.
    // Each agent runs its `run` method in a separate Tokio task.
    // Returns a `JoinHandle` for a supervisor task that waits for the shutdown
    // signal and then waits for all individual agent tasks in the group to finish.
    fn run_agent<T, F>(&self, num_tasks: usize, task_factory: F) -> JoinHandle<()>
    where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize, Weak<Broker>) -> T,
    {
        let broker = self.broker.as_ref().expect("broker should exist").clone();
        let agents = (0..num_tasks)
            .map(|i| Arc::new(tokio::sync::Mutex::new(task_factory(i, Arc::downgrade(&broker)))))
            .collect::<Vec<_>>();

        let mut handles = Vec::with_capacity(num_tasks);

        for agent in &agents {
            let agent = agent.clone();
            let handle = tokio::spawn(async move {
                let mut agent = agent.lock().await;
                agent.run().await;
            });
            handles.push(handle);
        }

        let mut broker = self.broker.as_ref().expect("broker should exist").clone();
        tokio::spawn(async move {
            broker.wait_for_shutdown().await;
            for (i, handle) in handles.into_iter().enumerate() {
                let agent = agents[i].clone();
                let name = {
                    let agent = agent.lock().await;
                    agent.name().to_string()
                };
                Self::wait_for_shutdown(TASK_SHUTDOWN_TIMEOUT_SECS, &name, handle).await;
            }
        })
    }

    // Waits for a specific task `handle` to shut down, with a given `wait_timeout`.
    // Logs the outcome (success, error, or timeout).
    async fn wait_for_shutdown<T>(
        wait_timeout: Duration,
        task_name: &str,
        handle: tokio::task::JoinHandle<T>,
    ) {
        tracing::debug!("waiting for {} shutdown", task_name);
        match timeout(wait_timeout, handle).await {
            Ok(result) => {
                if let Err(e) = result {
                    tracing::warn!("{} shutdown error: {}", task_name, e);
                } else {
                    tracing::debug!("{} shutdown completed successfully", task_name);
                }
            }
            Err(_) => {
                tracing::warn!(
                    "{} shutdown timed out after {} seconds",
                    task_name,
                    wait_timeout.as_secs()
                );
            }
        }
    }

    /// Add a task group supervisor task to the main tasks vector.
    fn add_task_group<T, F>(
        &self,
        tasks: &mut Vec<TaskHandle>,
        name: &str,
        num: usize,
        agent_type: F,
    ) where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize, Weak<Broker>) -> T,
    {
        tasks.push(TaskHandle {
            name: format!("{} Group", name),
            handle: self.run_agent::<T, F>(num, agent_type),
        });
    }
}

// Holds the name and JoinHandle for a task group supervisor.
struct TaskHandle {
    name: String,
    handle: JoinHandle<()>,
}
