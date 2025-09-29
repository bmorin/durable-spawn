//! A library for spawning durable tokio tasks that are automatically restarted on failure.
//!
//! This library provides a way to spawn long-running tasks that will be automatically
//! restarted if they unexpectedly complete, panic, become unresponsive, etc. Each task
//! is monitored by a watchdog that tracks heartbeats to detect hangs.
//!
//! # Example
//! ```
//! use durable_spawn::DurableSpawnBuilder;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = DurableSpawnBuilder::new(|watchdog| {
//!         let watchdog = watchdog.clone();
//!         async move {
//!             loop {
//!                 watchdog.heartbeat();
//!                 // Do some work...
//!                 tokio::time::sleep(Duration::from_secs(1)).await;
//!             }
//!         }
//!     })
//!     .heartbeat_timeout(Duration::from_secs(5))
//!     .spawn();
//!
//!     // Let it run for a while...
//!     tokio::time::sleep(Duration::from_secs(10)).await;
//!     handle.stop();
//! }
//! ```

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Default timeout for task heartbeats
const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

/// Statistics about a durable task's execution
#[derive(Debug, Clone, Copy)]
pub struct TaskStats {
    /// Number of times the task has been restarted
    pub restarts: i32,
    /// When the task was initially created
    pub created_at: Instant,
    /// Whether the task is currently running
    pub is_running: bool,
}

/// Internal watchdog state shared between the task and its monitor.
///
pub struct WatchdogInner {
    last_beat: AtomicI32,
    restarts: AtomicI32,
    is_running: AtomicBool,
    created_at: Instant,
}

impl WatchdogInner {
    /// Creates a new watchdog instance.
    fn new() -> Self {
        Self {
            last_beat: AtomicI32::new(0),
            restarts: AtomicI32::new(0),
            is_running: AtomicBool::new(true),
            created_at: Instant::now(),
        }
    }

    /// Records a heartbeat from the monitored task.
    ///
    /// This method *must* be called periodically by the task to indicate it's
    /// still alive and making progress. The watchdog monitor uses the frequency
    /// of these heartbeats to detect if the task has become unresponsive.
    ///
    /// This is an atomic operation and is safe to call from multiple threads,
    /// though typically only the monitored task should call this method.
    pub fn heartbeat(&self) {
        self.last_beat.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the current heartbeat counter value.
    ///
    /// This is used internally by the watchdog monitor to detect when a task
    /// has stopped sending heartbeats. The counter is incremented atomically
    /// each time `heartbeat()` is called.
    fn last_heartbeat(&self) -> i32 {
        self.last_beat.load(Ordering::Relaxed)
    }

    /// Signals the task to stop running.
    ///
    /// This sets the internal running flag to false, which will cause
    /// `should_continue()` to return false. The watchdog monitor checks
    /// this flag to determine when to stop monitoring and restart loops.
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    /// Checks if the task should continue running.
    ///
    /// Returns `true` if the task should keep running, `false` if it has been
    /// requested to stop via `stop()`. Tasks should check this periodically
    /// in their main loop to allow for graceful shutdown via TaskHandle::stop().
    pub fn should_continue(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Returns the TaskStats for the task.
    pub fn get_stats(&self) -> TaskStats {
        TaskStats {
            restarts: self.restarts.load(Ordering::Relaxed),
            created_at: self.created_at,
            is_running: self.is_running.load(Ordering::Relaxed),
        }
    }

    /// Increments the restart counter.
    ///
    /// This is called internally by the watchdog monitor each time it
    /// restarts the task due to a failure, panic, or hang. The counter
    /// is used for statistics and debugging purposes.
    fn record_restart(&self) {
        self.restarts.fetch_add(1, Ordering::Relaxed);
    }
}

/// A thread-safe handle to a watchdog instance.
///
/// This is an `Arc<WatchdogInner>` that allows the watchdog state to be shared
/// between the monitored task and the watchdog monitor. Tasks receive this handle
/// and use it to send heartbeats and check if they should continue running.
pub type Watchdog = Arc<WatchdogInner>;

/// A handle to control and monitor a durable task.
///
/// This handle is returned when spawning a durable task and provides methods to:
/// - Stop the task gracefully
/// - Get execution statistics (restart count, uptime, etc.)
/// - Check the current running status
///
/// The handle maintains references to the watchdog state and the current task's
/// abort handle, allowing it to both signal shutdown and forcefully abort the
/// running task if needed.
///
/// # Thread Safety
///
/// This handle is safe to share between threads and can be used to control
/// the task from anywhere in your application.
///
/// # Example
///
/// ```rust
/// use durable_spawn::DurableSpawnBuilder;
/// use std::time::Duration;
///
/// # #[tokio::main]
/// # async fn main() {
/// let handle = DurableSpawnBuilder::new(|watchdog| {
///     let watchdog = watchdog.clone();
///     async move {
///         loop {
///             watchdog.heartbeat();
///             tokio::time::sleep(Duration::from_secs(1)).await;
///         }
///     }
/// }).spawn();
///
/// // Get statistics
/// let stats = handle.stats();
/// println!("Task has been restarted {} times", stats.restarts);
///
/// // Stop the task
/// handle.stop();
/// # }
/// ```
pub struct TaskHandle {
    watchdog: Watchdog,
    current_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
}

impl TaskHandle {
    /// Stops the durable task.
    ///
    /// This method performs a graceful shutdown by:
    /// 1. Setting the watchdog's running flag to false, which signals the task
    ///    to stop during its next `should_continue()` check
    /// 2. Aborting the current task instance if one is running
    ///
    /// After calling this method, the watchdog monitor will stop restarting
    /// the task, and any currently running task instance will be aborted.
    ///
    /// This method is safe to call multiple times and from multiple threads.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use durable_spawn::DurableSpawnBuilder;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = DurableSpawnBuilder::new(|watchdog| {
    ///     // ... task implementation
    /// # async {}
    /// }).spawn();
    ///
    /// // Later, stop the task
    /// handle.stop();
    /// # }
    /// ```
    pub fn stop(&self) {
        self.watchdog.stop();
        // Also abort the current inner task if it exists
        if let Ok(mut current) = self.current_task.try_lock() {
            if let Some(task) = current.take() {
                task.abort();
            }
        }
    }

    /// Returns current statistics about the durable task.
    ///
    /// This method provides a snapshot of the task's execution history and
    /// current state, including:
    /// - `restarts`: The number of times the task has been restarted
    /// - `created_at`: When the task was originally created
    /// - `is_running`: Whether the task is currently marked as running
    ///
    /// The statistics are read atomically to ensure consistency.
    ///
    /// # Returns
    ///
    /// A [`TaskStats`] struct containing the current statistics.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use durable_spawn::DurableSpawnBuilder;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = DurableSpawnBuilder::new(|watchdog| {
    ///     // ... task implementation
    /// # async {}
    /// }).spawn();
    ///
    /// // Get current statistics
    /// let stats = handle.stats();
    /// println!("Task created at: {:?}", stats.created_at);
    /// println!("Restart count: {}", stats.restarts);
    /// println!("Currently running: {}", stats.is_running);
    /// # }
    /// ```
    pub fn stats(&self) -> TaskStats {
        self.watchdog.get_stats()
    }
}

/// Builder for configuring and spawning durable tasks
#[must_use = "builders do nothing unless spawned"]
pub struct DurableSpawnBuilder<F, Fut>
where
    F: Fn(&Watchdog) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    task_creator: F,
    heartbeat_timeout: Duration,
    delayed_start: Option<Duration>,
    maximum_restart_frequency: Option<Duration>,
}

impl<F, Fut> DurableSpawnBuilder<F, Fut>
where
    F: Fn(&Watchdog) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    /// Creates a new builder for a durable task.
    ///
    /// # Arguments
    ///
    /// * `task_creator` - A function that takes a watchdog handle and returns a future.
    ///   The future must call `watchdog.heartbeat()` periodically to indicate it's
    ///   still alive.
    pub fn new(task_creator: F) -> Self {
        Self {
            task_creator,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
            delayed_start: None,
            maximum_restart_frequency: None,
        }
    }

    /// Sets how long to wait for a heartbeat before considering the task hung
    /// and restarting it.
    ///
    /// # Arguments
    ///
    /// * `timeout` - The timeout duration. Must be positive and non-zero.
    ///
    /// # Panics
    ///
    /// Panics if the timeout is zero.
    pub fn heartbeat_timeout(mut self, timeout: impl Into<Duration>) -> Self {
        let timeout = timeout.into();
        assert!(!timeout.is_zero(), "heartbeat timeout must be non-zero");
        self.heartbeat_timeout = timeout;
        self
    }

    /// Sets a delay before the task is first started.
    ///
    /// # Arguments
    ///
    /// * `delay` - How long to wait before starting the task.
    pub fn delayed_start(mut self, delay: impl Into<Duration>) -> Self {
        self.delayed_start = Some(delay.into());
        self
    }

    /// Sets the minimum time between task restarts.
    ///
    /// Prevents rapid restart loops if a task is failing immediately after starting.
    ///
    /// # Arguments
    ///
    /// * `frequency` - The minimum time between restarts.
    pub fn maximum_restart_frequency(mut self, frequency: impl Into<Duration>) -> Self {
        self.maximum_restart_frequency = Some(frequency.into());
        self
    }

    /// Spawns the durable task.
    ///
    /// Returns a handle that can be used to stop the task and get statistics.
    #[must_use = "spawned task will be immediately dropped if the handle is not used"]
    pub fn spawn(self) -> TaskHandle {
        let task_creator = self.task_creator;
        let heartbeat_timeout = self.heartbeat_timeout;
        let watchdog = Arc::new(WatchdogInner::new());
        let current_task = Arc::new(Mutex::new(None));

        // watchdog and current_task are captured by the watchdog task below, so clone them now
        let task_handle = TaskHandle {
            watchdog: watchdog.clone(),
            current_task: current_task.clone(),
        };

        let _watchdog_task = tokio::spawn(async move {
            let mut last_beat = 0;
            let mut last_restart: Option<Instant> = None;
            let mut initial_start = true;

            if let Some(delay) = self.delayed_start {
                tokio::time::sleep(delay).await;
            }

            while watchdog.should_continue() {
                // Check if we need to wait before restarting
                if let Some(last) = last_restart {
                    if let Some(freq) = self.maximum_restart_frequency {
                        let elapsed = last.elapsed();
                        if elapsed < freq {
                            tracing::trace!("Waiting {:?} before restart...", freq - elapsed);
                            tokio::time::sleep(freq - elapsed).await;
                        }
                    }
                }

                if initial_start {
                    initial_start = false;
                } else {
                    watchdog.record_restart();
                    last_restart = Some(Instant::now());
                }

                let watchdog_ref = Arc::clone(&watchdog);
                let mut task_handle = tokio::spawn(task_creator(&watchdog_ref));

                // Store the current task so it can be aborted if needed
                {
                    let mut current = current_task.lock().await;
                    *current = Some(task_handle.abort_handle());
                }

                loop {
                    let current_beat = watchdog.last_heartbeat();

                    tokio::select! {
                        _ = tokio::time::sleep(heartbeat_timeout) => {
                            if current_beat == last_beat {
                                tracing::warn!("Task unresponsive, restarting...");
                                task_handle.abort();
                                break;
                            }
                            last_beat = current_beat;
                        }
                        result = &mut task_handle => {
                            match result {
                                Ok(_) => {
                                    tracing::warn!("Task completed, restarting...");
                                    break;
                                }
                                Err(e) => {
                                    tracing::error!("Task failed with error: {}, restarting...", e);
                                    break;
                                }
                            }
                        }
                    }
                }

                // Clean up the abort handle when we're done with this task
                let mut current = current_task.lock().await;
                current.take();
            }
        });

        task_handle
    }
}
