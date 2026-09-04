//! Provider-neutral durable memory port. Implementations own storage and network I/O.
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MemoryError(pub String);

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn recall(&self, query: &str) -> Result<Vec<String>, MemoryError>;
    async fn retain(&self, input: &str, answer: &str) -> Result<(), MemoryError>;
}
