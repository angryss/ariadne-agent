use std::collections::BTreeMap;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionDelta {
    Thinking(String),
    Content(String),
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

    async fn complete_stream(
        &self,
        request: CompletionRequest,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Completion, ProviderError> {
        let completion = self.complete(request).await?;
        on_delta(&CompletionDelta::Content(
            completion.message.content.clone(),
        ));
        Ok(completion)
    }
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
        let mut ignore_delta = |_: &CompletionDelta| {};
        self.respond_with(history, input, false, &mut ignore_delta)
            .await
    }

    pub async fn respond_stream(
        &self,
        history: &[Message],
        input: &str,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Message, AgentError> {
        self.respond_with(history, input, true, on_delta).await
    }

    async fn respond_with(
        &self,
        history: &[Message],
        input: &str,
        stream: bool,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Message, AgentError> {
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

        let request = CompletionRequest { messages };
        let completion = if stream {
            self.provider.complete_stream(request, on_delta).await?
        } else {
            self.provider.complete(request).await?
        };
        if completion.message.role != Role::Assistant {
            return Err(AgentError::InvalidProviderResponse);
        }
        Ok(completion.message)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Profile {
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub active_skills: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile name must not be blank")]
    BlankName,
    #[error("profile `{0}` is defined more than once")]
    Duplicate(String),
    #[error("default profile `{0}` is not defined")]
    UnknownDefault(String),
}

#[derive(Debug, Error)]
pub enum ProfileAgentError {
    #[error("profile `{0}` is not defined")]
    UnknownProfile(String),
    #[error(transparent)]
    Agent(#[from] AgentError),
}

#[derive(Clone)]
pub struct AgentProfiles {
    default_profile: Arc<str>,
    profiles: Arc<BTreeMap<String, (Profile, Agent)>>,
}

impl AgentProfiles {
    pub fn new(
        default_profile: impl Into<String>,
        profiles: impl IntoIterator<Item = (Profile, Agent)>,
    ) -> Result<Self, ProfileError> {
        let default_profile = default_profile.into();
        let mut indexed = BTreeMap::new();
        for (profile, agent) in profiles {
            if profile.name.trim().is_empty() {
                return Err(ProfileError::BlankName);
            }
            let name = profile.name.clone();
            if indexed.insert(name.clone(), (profile, agent)).is_some() {
                return Err(ProfileError::Duplicate(name));
            }
        }
        if !indexed.contains_key(&default_profile) {
            return Err(ProfileError::UnknownDefault(default_profile));
        }

        Ok(Self {
            default_profile: default_profile.into(),
            profiles: Arc::new(indexed),
        })
    }

    pub fn default_profile(&self) -> &str {
        self.default_profile.as_ref()
    }

    pub fn profiles(&self) -> Vec<Profile> {
        self.profiles
            .values()
            .map(|(profile, _)| profile.clone())
            .collect()
    }

    pub async fn respond(
        &self,
        profile: Option<&str>,
        history: &[Message],
        input: &str,
    ) -> Result<Message, ProfileAgentError> {
        let profile = profile.unwrap_or(self.default_profile.as_ref());
        let (_, agent) = self
            .profiles
            .get(profile)
            .ok_or_else(|| ProfileAgentError::UnknownProfile(profile.to_owned()))?;
        Ok(agent.respond(history, input).await?)
    }

    pub async fn respond_stream(
        &self,
        profile: Option<&str>,
        history: &[Message],
        input: &str,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Message, ProfileAgentError> {
        let profile = profile.unwrap_or(self.default_profile.as_ref());
        let (_, agent) = self
            .profiles
            .get(profile)
            .ok_or_else(|| ProfileAgentError::UnknownProfile(profile.to_owned()))?;
        Ok(agent.respond_stream(history, input, on_delta).await?)
    }
}
