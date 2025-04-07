use crate::agent::{Dispatcher, Reporter};
use crate::config;
use crate::message::Endpoint;
use crate::task::Runnable;
use crate::worker::Worker;
/// Controller module for the Pulse URL monitoring system
///
/// This module implements the Controller component which is the central coordinator
/// for the entire URL monitoring system. It manages the lifecycle of agents and workers,
/// handles communication channels, and ensures proper shutdown of the system.
use log::{debug, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::time;
use tokio::time::timeout;

const MONITOR_INTERVAL_SECS: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(10);
const TASK_SHUTDOWN_TIMEOUT_SECS: Duration = Duration::from_secs(1);

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
    shutdown_sender: watch::Sender<bool>,
}

impl Controller {
    /// Creates a new Controller with channels for request and response communication
    ///
    /// Initializes channels with a buffer size of 100 for both request and response channels
    pub fn new() -> Self {
        let (shutdown_sender, _) = watch::channel(false);

        Self { shutdown_sender }
    }

    /// Starts the monitoring system
    ///
    /// This method:
    /// 1. Spawns agent and worker tasks for parallel processing
    /// 2. Starts monitoring tasks to ensure agents and workers are running
    /// 3. Continuously receives and processes responses
    pub async fn start(&mut self) {
        // Create channels for request and response communication
        let (url_sender, url_receiver) = mpsc::channel::<Endpoint>(100);

        // Create channels for result communication
        let (result_sender, result_receiver) = mpsc::channel::<Endpoint>(100);

        // Create Arc wrappers for the receivers to allow sharing across threads
        let url_receiver = Arc::new(Mutex::new(url_receiver));
        let result_receiver = Arc::new(Mutex::new(result_receiver));

        // Get a receiver for shutdown signals
        let shutdown_receiver = self.shutdown_sender.subscribe();

        // Start dispatcher, reporter and worker group in parallel
        let reporter_handle = self.run_reporter(result_receiver, shutdown_receiver.clone());
        let worker_handle =
            self.run_workers(result_sender, url_receiver, shutdown_receiver.clone());
        let dispatcher_handle = self.run_dispatcher(url_sender, shutdown_receiver);

        // Wait for SIGINT or SIGTERM to initiate shutdown
        let mut sigint_stream = signal(SignalKind::interrupt()).expect("watch SIGINT failed");
        let mut sigterm_stream = signal(SignalKind::terminate()).expect("watch SIGTERM failed");
        tokio::select! {
            _ = sigint_stream.recv() => {
                self.shutdown_sender.send(true).expect("Failed to send shutdown signal");
                info!("SIGINT received, shutdown initiated...");
            }
            _ = sigterm_stream.recv() => {
                self.shutdown_sender.send(true).expect("Failed to send shutdown signal");
                info!("SIGTERM received, shutdown initiated...");
            }
        }

        // Wait for all systems to complete (they should run indefinitely)
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Dispatcher Group", dispatcher_handle).await;
        info!("Dispatcher Group shutdown complete");
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Worker Group", worker_handle).await;
        info!("Worker Group shutdown complete");
        Self::wait_for_shutdown(SHUTDOWN_TIMEOUT_SECS, "Reporter Group", reporter_handle).await;
        info!("Reporter Group shutdown complete");

        info!("Shutdown complete");
    }

    /// Generic factory function for creating and running tasks
    fn create_and_run_tasks<T, F>(
        capacity: usize,
        task_factory: F,
        shutdown_receiver: watch::Receiver<bool>,
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
            Self::run_task(tasks, shutdown_receiver).await;
        })
    }

    /// Runs the dispatcher system
    ///
    /// This method:
    /// 1. Spawns the initial dispatchers
    /// 2. Sets up monitoring to ensure they keep running
    /// 3. Returns a handle to the monitoring task
    fn run_dispatcher(
        &self,
        url_sender: mpsc::Sender<Endpoint>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let shutdown_receiver_for_tasks = shutdown_receiver.clone();
        Self::create_and_run_tasks(
            config::instance().agent_num(),
            move |i| {
                Arc::new(Dispatcher::new(
                    i,
                    url_sender.clone(),
                    shutdown_receiver_for_tasks.clone(),
                ))
            },
            shutdown_receiver,
        )
    }

    /// Runs the reporter system
    ///
    /// This method:
    /// 1. Spawns the initial reporters
    /// 2. Sets up monitoring to ensure they keep running
    /// 3. Returns a handle to the monitoring task
    fn run_reporter(
        &self,
        result_receiver: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        Self::create_and_run_tasks(
            config::instance().agent_num(),
            move |i| Arc::new(Reporter::new(i, result_receiver.clone())),
            shutdown_receiver,
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
        shutdown_receiver: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        Self::create_and_run_tasks(
            config::instance().worker_num(),
            move |i| Arc::new(Worker::new(i, url_receiver.clone(), result_sender.clone())),
            shutdown_receiver,
        )
    }

    async fn run_task<T: Runnable + Send + Sync + 'static>(
        tasks: Vec<Arc<T>>,
        shutdown_receiver: watch::Receiver<bool>,
    ) {
        let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(tasks.len());

        for task in &tasks {
            let task = Arc::clone(task);
            let handle = tokio::spawn(async move {
                task.start().await;
            });
            handles.push(handle);
        }

        // Monitor and restart tasks if needed
        let mut interval = time::interval(MONITOR_INTERVAL_SECS);
        let mut shutdown = *shutdown_receiver.borrow();
        while !shutdown {
            interval.tick().await;
            shutdown = *shutdown_receiver.borrow();

            // Check task health and replace failed ones
            if !shutdown {
                for (i, task) in tasks.iter().enumerate() {
                    if handles[i].is_finished() {
                        let task = Arc::clone(task);
                        handles[i] = tokio::spawn(async move {
                            task.start().await;
                            debug!("task {} restarted", task.name());
                        });
                    }
                }
            }
        }

        for (i, handle) in handles.into_iter().enumerate() {
            debug!("waiting for {} to finish", tasks[i].name());
            Self::wait_for_shutdown(TASK_SHUTDOWN_TIMEOUT_SECS, tasks[i].name(), handle).await;
        }
    }

    /// Wait for a JoinHandle to complete with a timeout
    ///
    /// # Arguments
    /// * `timeout_secs` - Timeout duration in seconds
    /// * `task_name` - Name of the task for logging purposes
    /// * `handle` - JoinHandle to wait for
    async fn wait_for_shutdown<T>(
        wait_timeout: Duration,
        task_name: &str,
        handle: tokio::task::JoinHandle<T>,
    ) {
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
