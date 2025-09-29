use durable_spawn::DurableSpawnBuilder;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,durable_spawn=trace".into()),
        )
        .init();

    println!("=== Durable Spawn Panic Recovery Demo ===");
    println!(
        "This example demonstrates how durable tasks are automatically restarted after panics.\n"
    );
    info!("Starting panic recovery demo with tracing enabled");

    let panic_counter = Arc::new(AtomicU32::new(0));
    let panic_counter_clone = panic_counter.clone();

    // Create a task that panics after a few heartbeats
    info!("Creating durable task with 2-second heartbeat timeout");
    let handle = DurableSpawnBuilder::new(move |watchdog| {
        let watchdog = watchdog.clone();
        let counter = panic_counter_clone.clone();
        async move {
            let run_count = counter.fetch_add(1, Ordering::Relaxed);
            println!("🚀 Task started (attempt #{})", run_count + 1);
            info!("Task execution started (attempt #{})", run_count + 1);

            // Send a few heartbeats then panic
            for i in 0..3 {
                println!("💓 Heartbeat {} (attempt #{})", i + 1, run_count + 1);
                info!("Sending heartbeat {} for attempt #{}", i + 1, run_count + 1);
                watchdog.heartbeat();
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // Panic to demonstrate restart behavior
            println!("💥 Task about to panic (attempt #{})...", run_count + 1);
            error!(
                "Task is about to panic intentionally (attempt #{})",
                run_count + 1
            );
            panic!(
                "Intentional panic for testing restart behavior (attempt #{})",
                run_count + 1
            );
        }
    })
    .heartbeat_timeout(Duration::from_secs(2))
    .spawn();

    info!("Durable task spawned successfully");

    println!("Task spawned. Watching for panics and restarts...\n");
    info!("Starting monitoring loop for task restarts");

    // Let the task panic and restart several times
    for i in 1..=10 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let stats = handle.stats();
        let total_attempts = panic_counter.load(Ordering::Relaxed);

        println!(
            "📊 Status check #{}: {} restarts, {} total attempts, running: {}",
            i, stats.restarts, total_attempts, stats.is_running
        );

        info!(
            "Status check {}: restarts={}, attempts={}, running={}",
            i, stats.restarts, total_attempts, stats.is_running
        );

        // Stop after we've seen a few restarts
        if stats.restarts >= 3 {
            println!(
                "\n✅ Successfully demonstrated {} restarts!",
                stats.restarts
            );
            info!(
                "Successfully demonstrated {} restarts - stopping demo",
                stats.restarts
            );
            break;
        }
    }

    println!("\n🛑 Stopping the task...");
    info!("Stopping durable task");
    handle.stop();

    // Wait for clean shutdown
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Final statistics
    let final_stats = handle.stats();
    let total_attempts = panic_counter.load(Ordering::Relaxed);

    println!("📋 Final Statistics:");
    println!("   • Total restart count: {}", final_stats.restarts);
    println!("   • Total task attempts: {}", total_attempts);
    println!("   • Task running: {}", final_stats.is_running);
    println!(
        "   • Task lifetime: {:.2}s",
        final_stats.created_at.elapsed().as_secs_f64()
    );

    info!(
        "Final stats: restarts={}, attempts={}, running={}, lifetime={:.2}s",
        final_stats.restarts,
        total_attempts,
        final_stats.is_running,
        final_stats.created_at.elapsed().as_secs_f64()
    );

    if final_stats.restarts > 0 && total_attempts > 1 {
        println!("\n🎉 SUCCESS: Task was successfully restarted after panics!");
        info!(
            "SUCCESS: Panic recovery demo completed successfully with {} restarts",
            final_stats.restarts
        );
    } else {
        println!("\n❌ ISSUE: Expected to see restarts, but didn't observe the expected behavior.");
        warn!(
            "ISSUE: Expected to see restarts but observed {} restarts and {} attempts",
            final_stats.restarts, total_attempts
        );
    }
}
