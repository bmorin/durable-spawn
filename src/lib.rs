use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

pub type Watchdog = Arc<WatchdogInner>;

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

pub fn durable_spawn<F, Fut>(task_creator: F, heartbeat_timeout: Duration) -> ()
where
    F: Fn(&Watchdog) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let watchdog = Arc::new(WatchdogInner::new());
        let mut last_beat = 0;

        loop {
            let watchdog_ref = Arc::clone(&watchdog);
            let mut task_handle = tokio::spawn(task_creator(&watchdog_ref));

            loop {
                let current_beat = watchdog.last_heartbeat();

                tokio::select! {
                    _ = tokio::time::sleep(heartbeat_timeout) => {
                        if current_beat == last_beat {
                            println!("Task unresponsive, restarting...");
                            task_handle.abort();
                            break;
                        }
                        last_beat = current_beat;
                    }
                    result = &mut task_handle => {
                        match result {
                            Ok(_) => {
                                println!("Task completed, restarting...");
                                break;
                            }
                            Err(e) => {
                                println!("Task failed with error: {}, restarting...", e);
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    // Return type is () so we don't need to do anything with the JoinHandle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_works() {
        durable_spawn(
            |watchdog| {
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
            },
            Duration::from_secs(1),
        );
        tokio::time::sleep(Duration::from_millis(1000)).await;
        println!("Test completed");
    }
}
