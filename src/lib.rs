use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct WatchdogInner {
    last_beat: AtomicI32,
}

impl WatchdogInner {
    fn new() -> Self {
        Self {
            last_beat: AtomicI32::new(0),
        }
    }

    pub fn heartbeat(&self) {
        self.last_beat.fetch_add(1, Ordering::Relaxed);
    }

    fn last_heartbeat(&self) -> i32 {
        self.last_beat.load(Ordering::Relaxed)
    }
}

pub type Watchdog = Arc<WatchdogInner>;

pub struct DurableSpawnBuilder<F, Fut>
where
    F: Fn(&Watchdog) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    task_creator: F,
    heartbeat_timeout: Duration,
    deplayed_start: Option<Duration>,
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
            deplayed_start: None,
            maximum_restart_frequency: None,
        }
    }

    pub fn heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = timeout;
        self
    }

    pub fn delayed_start(mut self, delay: Duration) -> Self {
        self.deplayed_start = Some(delay);
        self
    }

    pub fn maximum_restart_frequency(mut self, frequency: Duration) -> Self {
        self.maximum_restart_frequency = Some(frequency);
        self
    }

    pub fn spawn(self) {
        let task_creator = self.task_creator;
        let heartbeat_timeout = self.heartbeat_timeout;
        tokio::spawn(async move {
            let watchdog = Arc::new(WatchdogInner::new());
            let mut last_beat = 0;
            let mut last_restart: Option<Instant> = None;

            if let Some(delay) = self.deplayed_start {
                tokio::time::sleep(delay).await;
            }

            loop {
                if let Some(min_freq) = self.maximum_restart_frequency {
                    if let Some(last) = last_restart {
                        let elapsed = last.elapsed();
                        if elapsed < min_freq {
                            let wait_duration = min_freq - elapsed;
                            tracing::info!(
                                "Waiting {:?} before restarting task to respect minimum restart frequency",
                                wait_duration
                            );
                            tokio::time::sleep(wait_duration).await;
                        }
                    }
                    last_restart = Some(Instant::now());
                }

                let watchdog_ref = Arc::clone(&watchdog);
                let mut task_handle = tokio::spawn(task_creator(&watchdog_ref));

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
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_works() {
        DurableSpawnBuilder::new(|watchdog| {
            let watchdog = watchdog.clone();
            async move {
                println!("Task started");
                for i in 0..5 {
                    println!("Iteration {}", i);
                    watchdog.heartbeat();
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                println!("Task finished");
            }
        })
        .heartbeat_timeout(Duration::from_secs(1))
        .spawn();

        tokio::time::sleep(Duration::from_millis(1000)).await;
        println!("Test completed");
    }
}
