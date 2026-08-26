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
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: String::new(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

#[derive(Debug, Error)]
#[error("tool failed: {message}")]
pub struct ToolError {
    message: String,
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Completion {
    pub message: Message,
}

impl Completion {
    pub fn new(message: Message) -> Self {
        Self { message }
    }

    pub fn with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self::new(Message::assistant_with_tool_calls(tool_calls))
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
    #[error("tool name must not be blank")]
    BlankToolName,
    #[error("tool `{0}` is defined more than once")]
    DuplicateTool(String),
    #[error("agent exceeded the maximum of {0} model turns")]
    ToolLoopLimit(usize),
    #[error("agent exceeded the maximum of {0} tool calls")]
    ToolCallLimit(usize),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[derive(Clone)]
pub struct Agent {
    provider: Arc<dyn ModelProvider>,
    system_prompt: Arc<str>,
    tools: Arc<BTreeMap<String, Arc<dyn Tool>>>,
}

const MAX_MODEL_TURNS: usize = 8;
const MAX_TOOL_CALLS: usize = 64;

impl Agent {
    pub fn new(provider: Arc<dyn ModelProvider>, system_prompt: impl Into<Arc<str>>) -> Self {
        Self {
            provider,
            system_prompt: system_prompt.into(),
            tools: Arc::new(BTreeMap::new()),
        }
    }

    pub fn with_tools(
        provider: Arc<dyn ModelProvider>,
        system_prompt: impl Into<Arc<str>>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<Self, AgentError> {
        let mut indexed = BTreeMap::new();
        for tool in tools {
            let name = tool.definition().name;
            if name.trim().is_empty() {
                return Err(AgentError::BlankToolName);
            }
            if indexed.insert(name.clone(), tool).is_some() {
                return Err(AgentError::DuplicateTool(name));
            }
        }
        Ok(Self {
            provider,
            system_prompt: system_prompt.into(),
            tools: Arc::new(indexed),
        })
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
        if history.iter().any(|message| {
            !matches!(message.role, Role::User | Role::Assistant)
                || !message.tool_calls.is_empty()
                || message.tool_call_id.is_some()
        }) {
            return Err(AgentError::InvalidHistory);
        }

        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(Message::system(self.system_prompt.as_ref()));
        messages.extend_from_slice(history);
        messages.push(Message::user(input));

        let tools = self
            .tools
            .values()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>();
        let mut tool_calls_used = 0;
        for turn in 0..MAX_MODEL_TURNS {
            let request = CompletionRequest {
                messages: messages.clone(),
                tools: tools.clone(),
            };
            let mut turn_deltas = Vec::new();
            let completion = if stream {
                let mut buffer_delta = |delta: &CompletionDelta| turn_deltas.push(delta.clone());
                self.provider
                    .complete_stream(request, &mut buffer_delta)
                    .await?
            } else {
                self.provider.complete(request).await?
            };
            if completion.message.role != Role::Assistant {
                return Err(AgentError::InvalidProviderResponse);
            }
            if completion.message.tool_calls.is_empty() {
                for delta in &turn_deltas {
                    on_delta(delta);
                }
                return Ok(completion.message);
            }
            if turn + 1 == MAX_MODEL_TURNS {
                return Err(AgentError::ToolLoopLimit(MAX_MODEL_TURNS));
            }
            if tool_calls_used + completion.message.tool_calls.len() > MAX_TOOL_CALLS {
                return Err(AgentError::ToolCallLimit(MAX_TOOL_CALLS));
            }
            tool_calls_used += completion.message.tool_calls.len();

            let tool_calls = completion.message.tool_calls.clone();
            messages.push(completion.message);
            for call in tool_calls {
                let result = match self.tools.get(&call.name) {
                    Some(tool) => tool
                        .execute(call.arguments)
                        .await
                        .unwrap_or_else(|error| serde_json::json!({"error": error.to_string()})),
                    None => serde_json::json!({"error": format!("unknown tool `{}`", call.name)}),
                };
                messages.push(Message::tool(call.id, result.to_string()));
            }
        }
        Err(AgentError::ToolLoopLimit(MAX_MODEL_TURNS))
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
    #[serde(default)]
    pub capabilities: Vec<String>,
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
