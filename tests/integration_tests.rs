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

#[tokio::test]
async fn test_restart_on_early_return() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, Ordering};

    // Counter to track how many times the task has been executed
    let execution_counter = Arc::new(AtomicI32::new(0));
    let execution_counter_clone = execution_counter.clone();

    // Create a task that returns early (completes instead of running indefinitely)
    let handle = DurableSpawnBuilder::new(move |watchdog| {
        let counter = execution_counter_clone.clone();
        let watchdog = watchdog.clone();
        async move {
            let execution_num = counter.fetch_add(1, Ordering::Relaxed) + 1;
            println!("Task execution #{} started", execution_num);

            // Send a heartbeat to show we're alive
            watchdog.heartbeat();

            // Sleep briefly to simulate some work, then return early
            tokio::time::sleep(Duration::from_millis(50)).await;

            println!("Task execution #{} finishing early", execution_num);
            // Task returns here instead of running indefinitely
        }
    })
    .heartbeat_timeout(Duration::from_secs(1))
    .maximum_restart_frequency(Duration::from_millis(100)) // Allow quick restarts for testing
    .spawn();

    // Let the task run and restart a few times
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check statistics - the task should have restarted multiple times
    let stats = handle.stats();
    assert!(stats.is_running, "Task should still be running");
    assert!(
        stats.restarts > 0,
        "Task should have restarted at least once due to early returns"
    );

    // Verify the task function was called multiple times
    let executions = execution_counter.load(Ordering::Relaxed);
    assert!(
        executions > 1,
        "Task should have been executed multiple times (actual: {})",
        executions
    );

    // The number of executions should be restarts + 1 (initial execution)
    assert_eq!(
        executions,
        stats.restarts + 1,
        "Execution count should match restart count + 1"
    );

    // Stop the task
    handle.stop();

    // Wait for task to finish
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check final state
    let final_stats = handle.stats();
    assert!(!final_stats.is_running, "Task should be stopped");

    println!(
        "Test completed - task executed {} times with {} restarts",
        execution_counter.load(Ordering::Relaxed),
        final_stats.restarts
    );
}
