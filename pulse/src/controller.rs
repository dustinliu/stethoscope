use crate::agent::Agent;
use crate::config;
use crate::message::Endpoint;
use crate::task::Runnable;
use crate::worker::Worker;
/// Controller module for the Pulse URL monitoring system
///
/// This module implements the Controller component which is the central coordinator
/// for the entire URL monitoring system. It manages the lifecycle of agents and workers,
/// handles communication channels, and ensures proper shutdown of the system.
use log::{debug, info, trace, warn};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, mpsc};
use tokio::time;
use tokio::time::timeout;

const MONITOR_INTERVAL_SECS: u64 = 5;
const SHUTDOWN_TIMEOUT_SECS: u64 = 10;

/// Controller that manages the URL monitoring system
///
/// Coordinates between agents and workers by:
/// 1. Managing channels for request distribution and response collection
/// 2. Spawning and managing agent and worker tasks
/// 3. Monitoring the health of agents and workers
///
/// # Fields
/// * `request_sender` - Channel for sending requests to workers
/// * `request_receiver` - Receiver for workers to get requests
/// * `response_sender` - Channel for sending responses to agents
/// * `response_receiver` - Receiver for agents to get responses
/// * `shutdown_flag` - Flag to indicate if the system should shut down
pub struct Controller {
    shutdown_flag: Arc<AtomicBool>,
}

impl Controller {
    /// Creates a new Controller with channels for request and response communication
    ///
    /// Initializes channels with a buffer size of 100 for both request and response channels
    pub fn new() -> Self {
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        Self { shutdown_flag }
    }

    /// Starts the monitoring system
    ///
    /// This method:
    /// 1. Spawns agent and worker tasks for parallel processing
    /// 2. Starts monitoring tasks to ensure agents and workers are running
    /// 3. Continuously receives and processes responses
    pub async fn start(&mut self) {
        let (url_sender, url_receiver) = mpsc::channel::<Endpoint>(100);
        let (result_sender, result_receiver) = mpsc::channel::<Endpoint>(100);
        let url_receiver = Arc::new(Mutex::new(url_receiver));
        let result_receiver = Arc::new(Mutex::new(result_receiver));

        // Start agent and worker systems in parallel
        let worker_handle = self.run_workers(result_sender, url_receiver);
        let agent_handle = self.run_agent(url_sender, result_receiver);

        // Wait for both systems to complete (they should run indefinitely)
        let mut sigint_stream = signal(SignalKind::interrupt()).expect("無法監聽 SIGINT");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("無法監聽 SIGTERM");
        tokio::select! {
            _ = sigint_stream.recv() => {
                self.shutdown_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                info!("SIGINT received, shutdown initiated...");
            }
            _ = sigterm_stream.recv() => {
                self.shutdown_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                info!("SIGTERM received, shutdown initiated...");
            }
        }

        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Worker System", worker_handle).await;
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Agent System", agent_handle).await;

        info!("Shutdown complete");
    }

    /// Generic factory function for creating and running tasks
    fn create_and_run_tasks<T, F>(
        capacity: usize,
        task_factory: F,
        shutdown_flag: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<()>
    where
        T: Runnable + Send + Sync + 'static,
        F: Fn(usize) -> Arc<T>,
    {
        let mut tasks = Vec::with_capacity(capacity);

        // Create initial tasks
        for i in 0..capacity {
            tasks.push(task_factory(i));
        }

        tokio::spawn(async move {
            Self::run_task(tasks, shutdown_flag).await;
        })
    }

    /// Runs the agent system
    ///
    /// This method:
    /// 1. Spawns the initial agent
    /// 2. Sets up monitoring to ensure it keeps running
    /// 3. Returns a handle to the monitoring task
    fn run_agent(
        &self,
        url_sender: mpsc::Sender<Endpoint>,
        result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    ) -> tokio::task::JoinHandle<()> {
        let url_sender = url_sender.clone();
        let result_receiver = result_receiver.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        Self::create_and_run_tasks(
            config::instance().agent_num(),
            move |i| {
                Arc::new(Agent::new(
                    i,
                    url_sender.clone(),
                    result_receiver.clone(),
                    shutdown_flag.clone(),
                ))
            },
            self.shutdown_flag.clone(),
        )
    }

    /// Runs the worker system
    ///
    /// This method:
    /// 1. Spawns the initial workers
    /// 2. Sets up monitoring to ensure they keep running
    /// 3. Returns a handle to the monitoring task
    fn run_workers(
        &mut self,
        result_sender: mpsc::Sender<Endpoint>,
        url_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    ) -> tokio::task::JoinHandle<()> {
        let result_sender = result_sender.clone();
        let url_receiver = url_receiver.clone();

        Self::create_and_run_tasks(
            config::instance().worker_num(),
            move |i| Arc::new(Worker::new(i, url_receiver.clone(), result_sender.clone())),
            self.shutdown_flag.clone(),
        )
    }

    async fn run_task<T: Runnable + Send + Sync + 'static>(
        tasks: Vec<Arc<T>>,
        shutdown_flag: Arc<AtomicBool>,
    ) {
        let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(tasks.len());

        for task in &tasks {
            let task = task.clone();
            let handle = tokio::spawn(async move {
                task.start().await;
            });
            handles.push(handle);
        }

        // Monitor and restart tasks if needed
        let mut interval = time::interval(Duration::from_secs(MONITOR_INTERVAL_SECS));
        let mut shutdown = shutdown_flag.load(std::sync::atomic::Ordering::Relaxed);
        while !shutdown {
            interval.tick().await;
            shutdown = shutdown_flag.load(std::sync::atomic::Ordering::Relaxed);

            // Check task health and replace failed ones
            if !shutdown {
                for (i, task) in tasks.iter().enumerate() {
                    if handles[i].is_finished() {
                        let task = task.clone();
                        handles[i] = tokio::spawn(async move {
                            task.start().await;
                            debug!("task {} restarted", task.name());
                        });
                    }
                }
            }
        }

        debug!("waiting for all tasks to finish");
        for (i, handle) in handles.into_iter().enumerate() {
            trace!("waiting for {} to finish", tasks[i].name());
            Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, tasks[i].name(), handle).await;
        }
    }

    /// Wait for a JoinHandle to complete with a timeout
    ///
    /// # Arguments
    /// * `timeout_secs` - Timeout duration in seconds
    /// * `task_name` - Name of the task for logging purposes
    /// * `handle` - JoinHandle to wait for
    async fn wait_for_shutdown<T>(
        timeout_secs: u64,
        task_name: &str,
        handle: tokio::task::JoinHandle<T>,
    ) {
        let shutdown_timeout = Duration::from_secs(timeout_secs);
        info!("Waiting for {} to shutdown...", task_name);

        match timeout(shutdown_timeout, handle).await {
            Ok(result) => {
                if let Err(e) = result {
                    warn!("{} shutdown error: {}", task_name, e);
                } else {
                    info!("{} shutdown completed successfully", task_name);
                }
            }
            Err(_) => {
                warn!(
                    "{} shutdown timed out after {} seconds",
                    task_name,
                    shutdown_timeout.as_secs()
                );
            }
        }
    }
}
