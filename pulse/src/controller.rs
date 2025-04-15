use crate::{
    agent::{Aggregator, Dispatcher, Runnable, Worker},
    broker::Broker,
    config::{self, Config},
};
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::watch,
    task::JoinHandle,
    time::{Duration, MissedTickBehavior, timeout},
};

const MONITOR_INTERVAL_SECS: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(10);
const TASK_SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(5);

struct Task {
    name: String,
    handle: JoinHandle<()>,
}
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
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    broker: Broker,
    config: &'static Config,
}

impl Controller {
    /// Creates a new Controller with channels for request and response communication
    ///
    /// Initializes channels with a buffer size of 100 for both request and response channels
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            shutdown_tx,
            shutdown_rx,
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
        let aggregator_handle = self.run_agent(self.config.aggregator_num(), None, Aggregator::new);
        let worker_handle = self.run_agent(self.config.worker_num(), None, Worker::new);
        let dispatcher_handle = self.run_agent(1, Some(MONITOR_INTERVAL_SECS), Dispatcher::new);

        // Wait for SIGINT or SIGTERM to initiate shutdown
        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        tokio::select! {
            _ = sigint_stream.recv() => {
                info!("SIGINT received, shutdown initiated...");
                self.shutdown_tx.send(true).expect("failed to send shutdown signal");
            }
            _ = sigterm_stream.recv() => {
                info!("SIGTERM received, shutdown initiated...");
                self.shutdown_tx.send(true).expect("failed to send shutdown signal");
            }
        }

        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Dispatcher group", dispatcher_handle).await;
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Worker group", worker_handle).await;
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Aggregator group", aggregator_handle).await;

        info!("All tasks shutdown complete");
    }

    fn run_agent<T, F>(
        &self,
        num_tasks: usize,
        delay: Option<Duration>,
        task_factory: F,
    ) -> JoinHandle<()>
    where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize, Broker) -> T,
    {
        let agents = (0..num_tasks)
            .map(|i| Arc::new(task_factory(i, self.broker.clone())))
            .collect::<Vec<_>>();

        let mut tasks = Vec::with_capacity(num_tasks);

        for agent in agents {
            let mut shutdown_rx = self.shutdown_rx.clone();
            let name = agent.name().to_string();

            let handle = tokio::spawn(async move {
                let mut interval = delay.map(|d| {
                    let mut interval = tokio::time::interval(d);
                    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    interval
                });

                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            break;
                        }
                        _ = agent.run() => {
                            if let Some(ref mut interval) = interval {
                                interval.tick().await;
                            }
                        }
                    }
                }
            });

            tasks.push(Task {
                name: name.to_string(),
                handle,
            });
        }

        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::spawn(async move {
            shutdown_rx
                .changed()
                .await
                .expect("failed to receive shutdown signal");
            for task in tasks {
                Self::wait_for_shutdown(TASK_SHUTDOWN_TIMEOUT_SECS, &task.name, task.handle).await;
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
