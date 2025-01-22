# Context

A Rust implementation of Go-style context propagation with async support.

## Overview

This crate provides a context propagation mechanism inspired by Go's `context` package. It allows you to carry deadlines, cancellation signals, and key-value pairs through your program's call stack.

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
rs_ctx = "0.1.0"
```

Then in your code, you can use it either directly:

```rust
use rs_ctx::{ Context, self as context };
```

## Features

- Context cancellation with explicit cancel function
- Deadline/timeout support
- Type-safe key-value storage with generic types
- Hierarchical context inheritance
- Async-first design with Tokio runtime support

## Dependencies

- `tokio` (with "full" features) for async runtime support
- `serde` and `bincode` for key serialization
- `futures` for async utilities
- `thiserror` for error handling

## Examples

### Basic Context Operations

```rust
use rs_ctx::{background, with_value, with_cancel, with_deadline, with_timeout};
// Or if using through jaxon:
// use jaxon::context::{background, with_value, with_cancel, with_deadline, with_timeout};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    // Create a background context
    let ctx = background();
    assert!(ctx.err().is_none());
    assert!(ctx.deadline().is_none());
    
    // Add a value to context
    let ctx = with_value(&ctx, "key", "value".to_string()).await;
    
    // Create a cancellable context
    let (ctx, cancel) = with_cancel(&ctx);
    
    // Create a context with deadline
    let deadline = Instant::now() + Duration::from_secs(5);
    let (ctx, _) = with_deadline(&ctx, deadline);
    
    // Create a context with timeout
    let (ctx, _) = with_timeout(&ctx, Duration::from_secs(1));
    
    // Get value from context
    if let Some(value) = ctx.value::<_, String>(&"key") {
        println!("Value: {}", value);
    }
    
    // Cancel the context
    cancel.cancel().await;
    assert_eq!(ctx.err(), Some(ContextError::Canceled));
}
```

### Type-Safe Value Storage

```rust
use serde::Serialize;

// Custom key type
#[derive(Hash, Eq, PartialEq, Clone, Serialize)]
struct BoolKey(String);

// Custom value type
#[derive(Clone)]
struct CustomValue(Vec<u32>);

async fn type_safe_example() {
    let ctx = background();
    
    // Store different types of values
    let ctx = with_value(&ctx, "str_key", "test".to_string()).await;
    let ctx = with_value(&ctx, 42_i32, 42_i64).await;
    let ctx = with_value(&ctx, BoolKey("bool".into()), true).await;
    
    // Retrieve values with correct types
    let str_value: Option<Arc<String>> = ctx.value(&"str_key");
    let int_value: Option<Arc<i64>> = ctx.value(&42_i32);
    let bool_value: Option<Arc<bool>> = ctx.value(&BoolKey("bool".into()));
    
    assert_eq!(str_value.map(|v| (*v).clone()), Some("test".to_string()));
    assert_eq!(int_value.map(|v| (*v).clone()), Some(42_i64));
    assert_eq!(bool_value.map(|v| (*v).clone()), Some(true));
}
```

### Context Inheritance

```rust
async fn inheritance_example() {
    // Create root context with a value
    let root = background();
    let root = with_value(&root, "root_key", "root_value").await;
    
    // Create child context with a value
    let child = with_value(&root, "child_key", "child_value").await;
    
    // Create grandchild context with a value and deadline
    let deadline = Instant::now() + Duration::from_secs(1);
    let (grandchild, _) = with_deadline(&child, deadline);
    let grandchild = with_value(&grandchild, "grandchild_key", "grandchild_value").await;
    
    // Values are inherited down the chain
    assert_eq!(grandchild.value::<_, String>(&"root_key").map(|v| (*v).clone()), 
              Some("root_value".into()));
    assert_eq!(grandchild.value::<_, String>(&"child_key").map(|v| (*v).clone()), 
              Some("child_value".into()));
    assert_eq!(grandchild.value::<_, String>(&"grandchild_key").map(|v| (*v).clone()), 
              Some("grandchild_value".into()));
    
    // But values are not inherited up
    assert_eq!(child.value::<_, String>(&"grandchild_key"), None);
    
    // Deadlines are inherited
    assert_eq!(grandchild.deadline(), Some(deadline));
    assert!(child.deadline().is_none());
}
```

### Using with Tokio Tasks

```rust
use tokio::sync::mpsc;

async fn task_example() {
    let ctx = background();
    let (ctx, cancel) = with_cancel(&ctx);
    
    // Spawn a task that uses the context
    let handle = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            tokio::select! {
                _ = ctx.done() => {
                    // Context was cancelled
                    "cancelled"
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    // Timeout (should not happen)
                    "timeout"
                }
            }
        }
    });
    
    // Cancel the context after a short delay
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel().await;
    
    assert_eq!(handle.await.unwrap(), "cancelled");
}
```

### Sharing Values Across Tasks

```rust
async fn shared_values_example() {
    let ctx = background();
    let ctx = with_value(&ctx, "shared_key", "shared_value").await;
    
    // Spawn multiple tasks that access the same context value
    let mut handles = vec![];
    for i in 0..3 {
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            if let Some(value) = ctx.value::<_, String>(&"shared_key") {
                format!("task_{}_got_{}", i, value.as_ref())
            } else {
                format!("task_{}_no_value", i)
            }
        }));
    }
    
    // All tasks see the same value
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        assert_eq!(result, format!("task_{}_got_shared_value", i));
    }
}
```

### Task Cancellation and Cleanup

```rust
async fn cleanup_example() {
    let ctx = background();
    let (ctx, cancel) = with_cancel(&ctx);
    
    // Channel to verify cleanup
    let (tx, mut rx) = mpsc::channel(1);
    
    // Spawn a task that performs cleanup on cancellation
    let handle = tokio::spawn({
        let ctx = ctx.clone();
        let tx = tx.clone();
        async move {
            let result = tokio::select! {
                _ = ctx.done() => {
                    // Perform cleanup
                    tx.send("cleanup").await.unwrap();
                    "cancelled"
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    "timeout"
                }
            };
            result
        }
    });
    
    // Cancel the context
    cancel.cancel().await;
    
    // Verify cleanup was performed
    assert_eq!(handle.await.unwrap(), "cancelled");
    assert_eq!(rx.recv().await.unwrap(), "cleanup");
}
```

## Part of the Jaxon Framework

This crate is one of the core components of the Jaxon framework. Each component is published as a separate crate for flexibility:

- `jaxon` - The main framework crate
- `jaxon-context` - Context propagation (this crate)
- `jaxon-runtime` - Async runtime utilities
- `jaxon-macros` - Procedural macros for the framework
- ... (other Jaxon components)

Each component can be used independently or together with other components. This modular design allows you to only include the functionality you need in your project.

## License

This project is licensed under the Apache License, Version 2.0 - see the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 