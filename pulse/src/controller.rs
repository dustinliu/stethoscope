use crate::{
    agent::{Dispatcher, Worker, reporter::Proxy},
    broker::Broker,
    config::{self, Config},
    runnable::{Runnable, TasksGroup},
};
use anyhow::Result;
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::watch,
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
    config: &'static Config,
}

impl Controller {
    /// Creates a new Controller instance.
    ///
    /// Initializes the `Broker` and loads the application configuration.
    pub fn new() -> Self {
        Self {
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
        let mut groups = Vec::new();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let broker = Arc::new(Broker::new());

        let mut proxy_tasks = TasksGroup::new("Report Proxy Group");
        self.add_task_group::<Proxy, _>(
            broker.clone(),
            &mut proxy_tasks,
            AGGREGATOR_NUM,
            Proxy::new,
        );
        groups.push(proxy_tasks);

        let mut worker_tasks = TasksGroup::new("Worker Group");
        self.add_task_group::<Worker, _>(
            broker.clone(),
            &mut worker_tasks,
            self.config.worker.num_instance,
            |id, broker_weak| Worker::new(id, broker_weak, shutdown_rx.clone()),
        );
        groups.push(worker_tasks);

        let mut dispatcher_tasks = TasksGroup::new("Dispatcher Group");
        self.add_task_group::<Dispatcher, _>(
            broker.clone(),
            &mut dispatcher_tasks,
            DISPATCHER_NUM,
            |id, broker_weak| Dispatcher::new(id, broker_weak, shutdown_rx.clone()),
        );
        groups.push(dispatcher_tasks);

        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        tokio::select! {
            _ = sigint_stream.recv() => {
                tracing::info!("SIGINT received, shutdown initiated...");
                let _ = shutdown_tx.send(true);
            }
            _ = sigterm_stream.recv() => {
                tracing::info!("SIGTERM received, shutdown initiated...");
                let _ = shutdown_tx.send(true);
            }
        }
        drop(broker);

        for mut group in groups.into_iter().rev() {
            group
                .wait_for_shutdown(TASK_SHUTDOWN_TIMEOUT_SECS, SHUTDOWN_TIMEOUT_SECS)
                .await;
        }

        tracing::info!("All tasks shutdown complete");
    }

    /// Add a task group supervisor task to the main tasks vector.
    fn add_task_group<T, F>(
        &self,
        broker: Arc<Broker>,
        tasks: &mut TasksGroup,
        num: usize,
        agent_factory: F,
    ) where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize, Weak<Broker>) -> Result<T>,
    {
        for i in 0..num {
            match agent_factory(i, Arc::downgrade(&broker)) {
                Ok(agent) => tasks.add_task(Box::new(agent)),
                Err(e) => tracing::error!("failed to create agent: {}", e),
            }
        }
        tasks.run();
    }
}
