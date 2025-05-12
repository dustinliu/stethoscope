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
/// * `shutdown_tx` - A channel to signal shutdown.
/// * `config` - Static reference to the application configuration.
pub struct Controller {
    broker: Arc<Broker>,
    shutdown_tx: Option<watch::Sender<bool>>,
    config: &'static Config,
}

impl Controller {
    /// Creates a new Controller instance.
    ///
    /// Initializes the `Broker` using configuration and prepares shutdown channel.
    pub fn new() -> Self {
        let config = config::instance();
        let broker = Arc::new(Broker::new(config));
        Self {
            broker,
            shutdown_tx: None,
            config,
        }
    }

    /// Starts the monitoring system and waits for termination signals.
    ///
    /// This method:
    /// 1. Takes ownership of shutdown_tx from self.
    /// 2. Spawns task groups using the stored broker.
    /// 3. Waits for SIGINT or SIGTERM.
    /// 4. Initiates graceful shutdown via the shutdown channel.
    /// 5. Waits for all spawned tasks to complete their shutdown sequence.
    pub async fn start(mut self) {
        let mut groups = Vec::new();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let mut proxy_tasks = TasksGroup::new("Report Proxy Group");
        self.add_task_group::<Proxy, _>(
            self.broker.clone(),
            &mut proxy_tasks,
            AGGREGATOR_NUM,
            Proxy::new,
        );
        groups.push(proxy_tasks);

        let mut worker_tasks = TasksGroup::new("Worker Group");
        self.add_task_group::<Worker, _>(
            self.broker.clone(),
            &mut worker_tasks,
            self.config.worker.num_instance,
            |id, broker_weak| Worker::new(id, broker_weak, shutdown_rx.clone()),
        );
        groups.push(worker_tasks);

        let mut dispatcher_tasks = TasksGroup::new("Dispatcher Group");
        self.add_task_group::<Dispatcher, _>(
            self.broker.clone(),
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
                if let Some(tx) = self.shutdown_tx.as_ref() {
                    let _ = tx.send(true);
                }
            }
            _ = sigterm_stream.recv() => {
                tracing::info!("SIGTERM received, shutdown initiated...");
                if let Some(tx) = self.shutdown_tx.as_ref() {
                    let _ = tx.send(true);
                }
            }
        }
        drop(self.broker);

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
