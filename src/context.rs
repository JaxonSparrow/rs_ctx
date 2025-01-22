use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};

use super::{TypedKey, ContextTrait};

/// Context error types
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ContextError {
    #[error("context canceled")]
    Canceled,
    #[error("deadline exceeded")]
    DeadlineExceeded,
} 


/// Base context implementation
pub struct ContextImpl {
    /// Values stored in this context
    values: Arc<RwLock<HashMap<TypedKey, Arc<dyn Any + Send + Sync>>>>,
    /// Parent context if any
    parent: Option<Arc<Self>>,
    /// Cancel signal sender
    cancel_tx: broadcast::Sender<()>,
    /// Deadline if set
    deadline: Option<Instant>,
    /// Error if context is done
    error: Arc<RwLock<Option<ContextError>>>,
}

impl ContextImpl {
    /// Create new empty context
    pub(crate) fn background() -> Arc<Self> {
        let (tx, _) = broadcast::channel(1);
        Arc::new(Self {
            values: Arc::new(RwLock::new(HashMap::new())),
            parent: None,
            cancel_tx: tx,
            deadline: None,
            error: Arc::new(RwLock::new(None)),
        })
    }
    
    /// Create new context with cancel
    pub(crate) fn with_cancel(parent: &Arc<Self>) -> (Arc<Self>, CancelFn) {
        let (tx, _) = broadcast::channel(1);
        let ctx = Arc::new(Self {
            values: Arc::new(RwLock::new(HashMap::new())),
            parent: Some(parent.clone()),
            cancel_tx: tx.clone(),
            deadline: None,
            error: Arc::new(RwLock::new(None)),
        });
        
        let cancel_fn = CancelFn {
            tx: tx.clone(),
            error: ctx.error.clone(),
        };
        
        (ctx, cancel_fn)
    }
    
    /// Create new context with deadline
    pub(crate) fn with_deadline(
        parent: &Arc<Self>,
        deadline: Instant,
    ) -> (Arc<Self>, CancelFn) {
        let (tx, _) = broadcast::channel(1);
        let ctx = Arc::new(Self {
            values: Arc::new(RwLock::new(HashMap::new())),
            parent: Some(parent.clone()),
            cancel_tx: tx.clone(),
            deadline: Some(deadline),
            error: Arc::new(RwLock::new(None)),
        });
        
        let cancel_fn = CancelFn {
            tx: tx.clone(),
            error: ctx.error.clone(),
        };
        
        // Start deadline timer
        let error = ctx.error.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline.into()).await;
            let mut error = error.write().await;
            if error.is_none() {
                *error = Some(ContextError::DeadlineExceeded);
                let _ = tx.send(());
            }
        });
        
        (ctx, cancel_fn)
    }
    
    /// Create new context with timeout
    pub(crate) fn with_timeout(
        parent: &Arc<Self>,
        timeout: Duration,
    ) -> (Arc<Self>, CancelFn) {
        Self::with_deadline(parent, Instant::now() + timeout)
    }
    
    /// Create new context with value
    pub(crate) async fn with_value<K: Hash + Eq + Clone + Send + Sync + serde::Serialize + 'static, V: Any + Send + Sync>(
        parent: &Arc<Self>,
        key: K,
        value: V,
    ) -> Arc<Self> {
        let values = Arc::new(RwLock::new(HashMap::new()));
        {
            let mut values = values.write().await;
            values.insert(TypedKey::new(&key), Arc::new(value) as Arc<dyn Any + Send + Sync>);
        }
        
        let (tx, _) = broadcast::channel(1);
        Arc::new(Self {
            values,
            parent: Some(parent.clone()),
            cancel_tx: tx,
            deadline: parent.deadline,
            error: Arc::new(RwLock::new(None)),
        })
    }

    /// Get value from this context or its parents
    async fn get_value<K: Hash + Eq + Clone + Send + Sync + serde::Serialize + 'static, V: Any + Send + Sync + Clone>(&self, key: &K) -> Option<Arc<V>> {
        // Check current context's values
        let values = self.values.read().await;
        if let Some(value) = values.iter()
            .find(|(k, _)| k.matches(key))
            .and_then(|(_, v)| v.downcast_ref::<V>().map(|val| Arc::new((*val).clone()))) {
            return Some(value);
        }
        
        // Check parent context if any
        if let Some(parent) = &self.parent {
            let parent = parent.clone();
            let key = key.clone();
            let future = Box::pin(async move {
                parent.get_value(&key).await
            });
            return future.await;
        }
        
        None
    }
}

impl ContextTrait for ContextImpl {
    fn done(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let mut rx = self.cancel_tx.subscribe();
        Box::pin(async move {
            let _ = rx.recv().await;
        })
    }
    
    fn err(&self) -> Option<ContextError> {
        futures::executor::block_on(async {
            self.error.read().await.clone()
        })
    }
    
    fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn value<K: Hash + Eq + Clone + Send + Sync + serde::Serialize + 'static, V: Any + Send + Sync + Clone>(&self, key: &K) -> Option<Arc<V>> {
        futures::executor::block_on(async {
            self.get_value(key).await
        })
    }
}

/// Cancel function for canceling contexts
#[derive(Clone)]
pub struct CancelFn {
    tx: broadcast::Sender<()>,
    error: Arc<RwLock<Option<ContextError>>>,
}

impl CancelFn {
    /// Cancel the context
    pub async fn cancel(&self) {
        let mut error = self.error.write().await;
        if error.is_none() {
            *error = Some(ContextError::Canceled);
            let _ = self.tx.send(());
        }
    }
}