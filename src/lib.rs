mod context;
use std::any::TypeId;
use std::any::Any;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::future::Future;
use std::time::{Duration, Instant};

pub use context::{ContextImpl, CancelFn, ContextError};

/// The Context type used throughout the crate
pub type Context = Arc<ContextImpl>;

/// A key with its type information
#[derive(Hash, Eq, PartialEq, Clone)]
pub(crate) struct TypedKey {
    /// The serialized key value
    key: Vec<u8>,
    /// Type ID of the key
    type_id: TypeId,
}

impl TypedKey {
    pub fn new<K: serde::Serialize + 'static>(key: &K) -> Self {
        Self {
            key: bincode::serialize(key).expect("Failed to serialize key"),
            type_id: TypeId::of::<K>(),
        }
    }

    pub fn matches<K: serde::Serialize + 'static>(&self, key: &K) -> bool {
        if self.type_id != TypeId::of::<K>() {
            return false;
        }
        
        let key_bytes = bincode::serialize(key).expect("Failed to serialize key");
        self.key == key_bytes
    }
} 

/// Base context trait defining core functionality
pub trait ContextTrait: Send + Sync {
    /// Returns a future that completes when the context is done
    fn done(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    
    /// Returns error if context is canceled or deadline exceeded
    fn err(&self) -> Option<ContextError>;
    
    /// Returns deadline if one is set
    fn deadline(&self) -> Option<Instant>;

    /// Returns value for key if it exists
    fn value<K: Hash + Eq + Clone + Send + Sync + serde::Serialize + 'static, V: Any + Send + Sync + Clone>(&self, key: &K) -> Option<Arc<V>>;
}

/// Create background context
pub fn background() -> Context {
    ContextImpl::background()
}

/// Create context with cancel
pub fn with_cancel(parent: &Context) -> (Context, CancelFn) {
    ContextImpl::with_cancel(parent)
}

/// Create context with deadline
pub fn with_deadline(
    parent: &Context,
    deadline: Instant,
) -> (Context, CancelFn) {
    ContextImpl::with_deadline(parent, deadline)
}

/// Create context with timeout
pub fn with_timeout(
    parent: &Context,
    timeout: Duration,
) -> (Context, CancelFn) {
    ContextImpl::with_timeout(parent, timeout)
}

/// Create context with value
pub async fn with_value<K: Hash + Eq + Clone + Send + Sync + serde::Serialize + 'static, V: Any + Send + Sync>(
    parent: &Context,
    key: K,
    value: V,
) -> Context {
    ContextImpl::with_value(parent, key, value).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use serde::Serialize;
    use std::sync::Arc;
    use std::time::Instant;
    
    #[tokio::test]
    async fn test_background() {
        let ctx = background();
        assert!(ctx.err().is_none());
        assert!(ctx.deadline().is_none());
    }
    
    #[tokio::test]
    async fn test_cancel() {
        let ctx = background();
        let (ctx, cancel) = with_cancel(&ctx);
        
        assert!(ctx.err().is_none());
        
        cancel.cancel().await;
        
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(ctx.err(), Some(ContextError::Canceled));
    }
    
    #[tokio::test]
    async fn test_deadline() {
        let ctx = background();
        let deadline = Instant::now() + Duration::from_millis(50);
        let (ctx, _) = with_deadline(&ctx, deadline);
        
        assert!(ctx.err().is_none());
        assert_eq!(ctx.deadline(), Some(deadline));
        
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(ctx.err(), Some(ContextError::DeadlineExceeded));
    }
    
    #[tokio::test]
    async fn test_value() {
        let ctx = background();
        let ctx = with_value(&ctx, "test_key", "test_value".to_string()).await;
        
        let value: Option<Arc<String>> = ctx.value(&"test_key");
        assert_eq!(value.map(|v| (*v).clone()), Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_multiple_values() {
        let ctx = background();
        
        // Add string value with string key
        let ctx = with_value(&ctx, "str_key", "test".to_string()).await;
        
        // Add integer value with integer key
        let ctx = with_value(&ctx, 42_i32, 42_i64).await;
        
        // Add boolean value with custom key
        #[derive(Hash, Eq, PartialEq, Clone, Serialize)]
        struct BoolKey(String);
        let ctx = with_value(&ctx, BoolKey("bool".to_string()), true).await;
        
        // Verify all values are accessible
        let str_value: Option<Arc<String>> = ctx.value(&"str_key");
        let int_value: Option<Arc<i64>> = ctx.value(&42_i32);
        let bool_value: Option<Arc<bool>> = ctx.value(&BoolKey("bool".to_string()));
        
        assert_eq!(str_value.map(|v| (*v).clone()), Some("test".to_string()));
        assert_eq!(int_value.map(|v| (*v).clone()), Some(42_i64));
        assert_eq!(bool_value.map(|v| (*v).clone()), Some(true));
    }

    #[tokio::test]
    async fn test_same_type_different_keys() {
        let ctx = background();
        
        // Add two string values with different string keys
        let ctx = with_value(&ctx, "key1", "first".to_string()).await;
        let ctx = with_value(&ctx, "key2", "second".to_string()).await;
        
        // Verify both values are accessible and distinct
        let value1: Option<Arc<String>> = ctx.value(&"key1");
        let value2: Option<Arc<String>> = ctx.value(&"key2");
        
        assert_eq!(value1.map(|v| (*v).clone()), Some("first".to_string()));
        assert_eq!(value2.map(|v| (*v).clone()), Some("second".to_string()));
    }

    #[tokio::test]
    async fn test_nested_contexts() {
        // Create root context with a value
        let root = background();
        let root = with_value(&root, "root_key", "root_value".to_string()).await;
        
        // Create child context with a value
        let child = with_value(&root, "child_key", "child_value".to_string()).await;
        
        // Create grandchild context with a value and deadline
        let deadline = Instant::now() + Duration::from_secs(1);
        let (grandchild, _) = with_deadline(&child, deadline);
        let grandchild = with_value(&grandchild, "grandchild_key", "grandchild_value".to_string()).await;
        
        // Verify values are accessible from grandchild context
        let root_value: Option<Arc<String>> = grandchild.value(&"root_key");
        let child_value: Option<Arc<String>> = grandchild.value(&"child_key");
        let grandchild_value: Option<Arc<String>> = grandchild.value(&"grandchild_key");
        
        assert_eq!(root_value.map(|v| (*v).clone()), Some("root_value".to_string()));
        assert_eq!(child_value.map(|v| (*v).clone()), Some("child_value".to_string()));
        assert_eq!(grandchild_value.map(|v| (*v).clone()), Some("grandchild_value".to_string()));
        
        // Verify values are accessible from child context
        let root_from_child: Option<Arc<String>> = child.value(&"root_key");
        let child_from_child: Option<Arc<String>> = child.value(&"child_key");
        let grandchild_from_child: Option<Arc<String>> = child.value(&"grandchild_key");
        
        assert_eq!(root_from_child.map(|v| (*v).clone()), Some("root_value".to_string()));
        assert_eq!(child_from_child.map(|v| (*v).clone()), Some("child_value".to_string()));
        assert_eq!(grandchild_from_child, None); // Cannot access grandchild's value from child
        
        // Verify deadline is inherited
        assert_eq!(grandchild.deadline(), Some(deadline));
        assert!(child.deadline().is_none());
    }

    #[tokio::test]
    async fn test_context_in_tokio_tasks() {
        let ctx = background();
        let (ctx, cancel) = with_cancel(&ctx);
        
        // Spawn a task that uses the context
        let handle = tokio::spawn({
            let ctx = ctx.clone();
            async move {
                // Create a future that completes when context is done
                let done = ctx.done();
                
                tokio::select! {
                    _ = done => {
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
        
        // Task should complete with "cancelled"
        let result = handle.await.unwrap();
        assert_eq!(result, "cancelled");
    }

    #[tokio::test]
    async fn test_context_value_across_tasks() {
        let ctx = background();
        let ctx = with_value(&ctx, "key", "parent_value".to_string()).await;
        
        // Spawn multiple tasks that access the same context value
        let mut handles = vec![];
        for i in 0..3 {
            let ctx = ctx.clone();
            handles.push(tokio::spawn(async move {
                if let Some(value) = ctx.value::<_, String>(&"key") {
                    format!("task_{}_got_{}", i, value.as_ref())
                } else {
                    format!("task_{}_no_value", i)
                }
            }));
        }
        
        // All tasks should get the same value
        for (i, handle) in handles.into_iter().enumerate() {
            let result = handle.await.unwrap();
            assert_eq!(result, format!("task_{}_got_parent_value", i));
        }
    }

    #[tokio::test]
    async fn test_context_deadline_in_tasks() {
        let ctx = background();
        let deadline = Instant::now() + Duration::from_millis(100);
        let (ctx, _) = with_deadline(&ctx, deadline);
        
        // Spawn a task that waits on context deadline
        let handle = tokio::spawn({
            let ctx = ctx.clone();
            async move {
                let done = ctx.done();
                
                tokio::select! {
                    _ = done => {
                        ctx.err().unwrap() == ContextError::DeadlineExceeded
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        false
                    }
                }
            }
        });
        
        // Task should complete with deadline exceeded
        let deadline_exceeded = handle.await.unwrap();
        assert!(deadline_exceeded);
    }

    #[tokio::test]
    async fn test_nested_tasks_with_context() {
        let ctx = background();
        
        // Create a chain of nested contexts with values
        let mut current_ctx = ctx;
        let mut expected_values = Vec::new();
        
        for i in 0..=3 {
            current_ctx = with_value(&current_ctx, format!("level_{}", i), i).await;
            expected_values.push(i);
            
            // Spawn a task to read all values up to this level
            let ctx = current_ctx.clone();
            let handle = tokio::spawn(async move {
                let mut values = Vec::new();
                for level in 0..=i {
                    if let Some(value) = ctx.value::<_, i32>(&format!("level_{}", level)) {
                        values.push(*value.as_ref());
                    }
                }
                values
            });
            
            let values = handle.await.unwrap();
            assert_eq!(values, expected_values);
        }
    }

    #[tokio::test]
    async fn test_task_cancellation_cleanup() {
        let ctx = background();
        let (ctx, cancel) = with_cancel(&ctx);
        
        // Channel to verify cleanup
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        
        // Spawn a task that performs cleanup on cancellation
        let handle = tokio::spawn({
            let ctx = ctx.clone();
            let tx = tx.clone();
            async move {
                let result = tokio::select! {
                    _ = ctx.done() => {
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
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel().await;
        
        // Verify task was cancelled and cleanup was performed
        assert_eq!(handle.await.unwrap(), "cancelled");
        assert_eq!(rx.recv().await.unwrap(), "cleanup");
    }
} 