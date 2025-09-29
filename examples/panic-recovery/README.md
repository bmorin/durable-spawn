# Panic Recovery Demo

This is a standalone console application that demonstrates the panic recovery capabilities of the `durable-spawn` library.

## What it does

This demo creates a durable task that:
1. Starts up and sends heartbeats
2. Intentionally panics after a few heartbeats
3. Gets automatically restarted by the durable-spawn system
4. Repeats this process to show consistent recovery behavior

## Running the demo

From the `examples/panic-recovery` directory, run:

```bash
cargo run
```

### With detailed tracing output

To see detailed tracing output from the durable-spawn library, set the `RUST_LOG` environment variable:

```bash
# Windows PowerShell
$env:RUST_LOG="debug,durable_spawn=trace"; cargo run

# Windows CMD
set RUST_LOG=debug,durable_spawn=trace && cargo run

# Linux/macOS
RUST_LOG=debug,durable_spawn=trace cargo run
```

This will show internal library operations like task restarts, heartbeat timeouts, and error handling.

## Expected output

You should see output similar to:

```
=== Durable Spawn Panic Recovery Demo ===
This example demonstrates how durable tasks are automatically restarted after panics.

🚀 Task started (attempt #1)
💓 Heartbeat 1 (attempt #1)
💓 Heartbeat 2 (attempt #1)
💓 Heartbeat 3 (attempt #1)
💥 Task about to panic (attempt #1)...
📊 Status check #1: 1 restarts, 2 total attempts, running: true
🚀 Task started (attempt #2)
💓 Heartbeat 1 (attempt #2)
💓 Heartbeat 2 (attempt #2)
💓 Heartbeat 3 (attempt #2)
💥 Task about to panic (attempt #2)...
📊 Status check #2: 2 restarts, 3 total attempts, running: true
...
✅ Successfully demonstrated 3 restarts!
🛑 Stopping the task...
📋 Final Statistics:
   • Total restart count: 3
   • Total task attempts: 4
   • Task running: false
   • Task lifetime: 8.52s

🎉 SUCCESS: Task was successfully restarted after panics!
```

## What this proves

- Tasks that panic are automatically detected and restarted
- The restart counter accurately tracks the number of restarts
- The system continues to function normally after panics
- The task can be cleanly shut down even after experiencing panics