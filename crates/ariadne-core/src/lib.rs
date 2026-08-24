use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Completion {
    pub message: Message,
}

impl Completion {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

#[derive(Debug, Error)]
#[error("model provider failed: {message}")]
pub struct ProviderError {
    message: String,
}

impl ProviderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<Completion, ProviderError>;
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("user input must not be blank")]
    BlankInput,
    #[error("conversation history must contain only user and assistant messages")]
    InvalidHistory,
    #[error("model provider response must contain an assistant message")]
    InvalidProviderResponse,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[derive(Clone)]
pub struct Agent {
    provider: Arc<dyn ModelProvider>,
    system_prompt: Arc<str>,
}

impl Agent {
    pub fn new(provider: Arc<dyn ModelProvider>, system_prompt: impl Into<Arc<str>>) -> Self {
        Self {
            provider,
            system_prompt: system_prompt.into(),
        }
    }

    pub async fn respond(&self, history: &[Message], input: &str) -> Result<Message, AgentError> {
        if input.trim().is_empty() {
            return Err(AgentError::BlankInput);
        }
        if history.iter().any(|message| message.role == Role::System) {
            return Err(AgentError::InvalidHistory);
        }

        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(Message::system(self.system_prompt.as_ref()));
        messages.extend_from_slice(history);
        messages.push(Message::user(input));

        let completion = self
            .provider
            .complete(CompletionRequest { messages })
            .await?;
        if completion.message.role != Role::Assistant {
            return Err(AgentError::InvalidProviderResponse);
        }
        Ok(completion.message)
    }
}
