use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct TaskStats {
    pub restarts: i32,
    pub created_at: Instant,
    pub is_running: bool,
}

pub struct WatchdogInner {
    last_beat: AtomicI32,
    restarts: AtomicI32,
    is_running: AtomicBool,
    created_at: Instant,
}

impl WatchdogInner {
    fn new() -> Self {
        Self {
            last_beat: AtomicI32::new(0),
            restarts: AtomicI32::new(0),
            is_running: AtomicBool::new(true),
            created_at: Instant::now(),
        }
    }

    pub fn heartbeat(&self) {
        self.last_beat.fetch_add(1, Ordering::Relaxed);
    }

    fn last_heartbeat(&self) -> i32 {
        self.last_beat.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    pub fn should_continue(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn get_stats(&self) -> TaskStats {
        TaskStats {
            restarts: self.restarts.load(Ordering::Relaxed),
            created_at: self.created_at,
            is_running: self.is_running.load(Ordering::Relaxed),
        }
    }

    fn record_restart(&self) {
        self.restarts.fetch_add(1, Ordering::Relaxed);
    }
}

pub type Watchdog = Arc<WatchdogInner>;

pub struct TaskHandle {
    watchdog: Watchdog,
    current_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
}

impl TaskHandle {
    pub fn stop(&self) {
        self.watchdog.stop();
        // Also abort the current inner task if it exists
        if let Ok(mut current) = self.current_task.try_lock() {
            if let Some(task) = current.take() {
                task.abort();
            }
        }
    }

    pub fn stats(&self) -> TaskStats {
        self.watchdog.get_stats()
    }
}

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
    pub fn new(task_creator: F) -> Self {
        Self {
            task_creator,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
            delayed_start: None,
            maximum_restart_frequency: None,
        }
    }

    pub fn heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = timeout;
        self
    }

    pub fn delayed_start(mut self, delay: Duration) -> Self {
        self.delayed_start = Some(delay);
        self
    }

    pub fn maximum_restart_frequency(mut self, frequency: Duration) -> Self {
        self.maximum_restart_frequency = Some(frequency);
        self
    }

    pub fn spawn(self) -> TaskHandle {
        let task_creator = self.task_creator;
        let heartbeat_timeout = self.heartbeat_timeout;
        let watchdog = Arc::new(WatchdogInner::new());
        let current_task = Arc::new(Mutex::new(None));

        // watchdog and current_task are captured by the watchdog task below
        let result = TaskHandle {
            watchdog: watchdog.clone(),
            current_task: current_task.clone(),
        };

        let _watchdog_task = tokio::spawn(async move {
            let mut last_beat = 0;

            if let Some(delay) = self.delayed_start {
                tokio::time::sleep(delay).await;
            }

            while watchdog.should_continue() {
                if last_beat != 0 {
                    // Assume last_beat is 0 on only the first run, so we don't count that as a restart
                    watchdog.record_restart();
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

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_control() {
        // Create a task that runs in a loop until stopped
        let handle = DurableSpawnBuilder::new(|watchdog| {
            let watchdog = watchdog.clone();
            async move {
                println!("Task started");
                while watchdog.should_continue() {
                    println!("Task heartbeat");
                    watchdog.heartbeat();
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                println!("Task finished");
            }
        })
        .heartbeat_timeout(Duration::from_secs(1))
        .spawn();

        // Let the task run for a bit
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Get statistics
        let stats = handle.stats();
        assert!(stats.is_running, "Task should be running");
        assert_eq!(stats.restarts, 0, "Task should not have restarted");
        assert!(
            stats.created_at.elapsed() > Duration::from_millis(400),
            "Task should have been running for at least 400ms"
        );

        // Stop the task
        handle.stop();

        // Wait for task to finish
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Check final state
        let final_stats = handle.stats();
        assert!(!final_stats.is_running, "Task should be stopped");
    }
}
