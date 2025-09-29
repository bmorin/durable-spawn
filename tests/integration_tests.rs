use durable_spawn::*;
use std::time::Duration;

#[tokio::test]
async fn test_simple() {
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
