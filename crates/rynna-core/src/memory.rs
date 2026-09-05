//! Provider-neutral durable memory and bounded background retention.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use thiserror::Error;
use tokio::{sync::oneshot, task::JoinHandle};
use uuid::Uuid;

use crate::{Message, Role};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MemoryError(pub String);

#[derive(Clone, Debug, Serialize)]
pub struct MemoryMessage {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Caller-visible history plus the newly completed exchange. Never model-only context.
#[derive(Clone, Debug)]
pub struct MemoryConversation {
    pub session_id: Uuid,
    pub messages: Vec<MemoryMessage>,
    pub timestamp: String,
}

impl MemoryConversation {
    pub(crate) fn completed(
        session_id: Uuid,
        history: &[Message],
        input: &str,
        answer: &str,
    ) -> Self {
        let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut messages: Vec<_> = history
            .iter()
            .map(|message| MemoryMessage {
                role: message.role.clone(),
                content: message.content.clone(),
                timestamp: None,
            })
            .collect();
        for (role, content) in [(Role::User, input), (Role::Assistant, answer)] {
            messages.push(MemoryMessage {
                role,
                content: content.into(),
                timestamp: Some(timestamp.clone()),
            });
        }
        Self {
            session_id,
            messages,
            timestamp,
        }
    }
}

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn recall(&self, query: &str) -> Result<Vec<String>, MemoryError>;
    async fn retain(&self, conversation: &MemoryConversation) -> Result<(), MemoryError>;
}

// Track accepted writes independently of live settings, so disabling/replacing a
// provider cannot abandon its queued work. Handles are pruned on each enqueue.
static PENDING_WRITES: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());
const MAX_PENDING_WRITES: usize = 128;
const MAX_CONVERSATION_BYTES: usize = 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct RetentionQueue {
    tail: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl RetentionQueue {
    pub fn enqueue(&self, provider: Arc<dyn MemoryProvider>, conversation: MemoryConversation) {
        if conversation
            .messages
            .iter()
            .map(|m| m.content.len())
            .sum::<usize>()
            > MAX_CONVERSATION_BYTES
        {
            tracing::warn!("memory retention skipped: conversation exceeds queue size limit");
            return;
        }
        let mut pending = PENDING_WRITES.lock().expect("memory write lock poisoned");
        pending.retain(|task| !task.is_finished());
        if pending.len() >= MAX_PENDING_WRITES {
            tracing::warn!("memory retention skipped: write queue is full");
            return;
        }
        let (done, next) = oneshot::channel();
        let previous = self
            .tail
            .lock()
            .expect("memory queue lock poisoned")
            .replace(next);
        pending.push(tokio::spawn(async move {
            // FIFO per provider snapshot, including failures. Other profiles can write concurrently.
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            if !matches!(
                tokio::time::timeout(Duration::from_secs(10), provider.retain(&conversation)).await,
                Ok(Ok(()))
            ) {
                tracing::warn!("memory retention unavailable; response was not saved to memory");
            }
            let _ = done.send(());
        }));
    }
}

/// Drain accepted writes before the host shuts down its async runtime. This waits
/// for HTTP acceptance, not server-side extraction. Forced exits cannot drain.
pub async fn flush_memory_writes() {
    let mut pending =
        std::mem::take(&mut *PENDING_WRITES.lock().expect("memory write lock poisoned"));
    if tokio::time::timeout(Duration::from_secs(10), async {
        for task in &mut pending {
            let _ = task.await;
        }
    })
    .await
    .is_err()
    {
        for task in pending {
            task.abort();
        }
        tracing::warn!("memory shutdown deadline reached; pending writes were cancelled");
    }
}
