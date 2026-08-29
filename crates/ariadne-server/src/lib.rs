use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use ariadne_config::{
    ConfiguredProvider, OpenAiAuthentication, ProviderSettingsError, ProviderSettingsStore,
    secure_private_directory,
};
use ariadne_core::{
    Agent, AgentError, AgentProfiles, CompletionDelta, Message, Profile, ProfileAgentError,
};
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Path as AxumPath, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{Next, from_fn};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, timeout};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    profiles: Arc<AgentProfiles>,
    provider_settings: Option<Arc<Mutex<ProviderSettingsStore>>>,
    codex_program: PathBuf,
    codex_home: PathBuf,
}

pub fn router(agent: Agent) -> Router {
    let profile = Profile {
        name: "default".to_owned(),
        provider: "configured".to_owned(),
        model: "configured".to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
        capabilities: Vec::new(),
    };
    let profiles = AgentProfiles::new("default", [(profile, agent)])
        .expect("the built-in server profile must be valid");
    router_with_profiles(profiles)
}

pub fn router_with_profiles(profiles: AgentProfiles) -> Router {
    router_with_state(profiles, None)
}

pub fn router_with_profiles_and_provider_settings(
    profiles: AgentProfiles,
    provider_settings: ProviderSettingsStore,
) -> Router {
    router_with_state(profiles, Some(provider_settings))
}

fn router_with_state(
    profiles: AgentProfiles,
    provider_settings: Option<ProviderSettingsStore>,
) -> Router {
    let codex_program = std::env::var_os("ARIADNE_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"));
    let codex_home = std::env::var_os("ARIADNE_CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|path| path.join("ariadne").join("codex")))
        .unwrap_or_else(|| PathBuf::from(".ariadne-codex"));
    router_with_runtime(profiles, provider_settings, codex_program, codex_home)
}

pub fn router_with_profiles_and_provider_runtime(
    profiles: AgentProfiles,
    provider_settings: ProviderSettingsStore,
    codex_program: impl Into<PathBuf>,
    codex_home: impl Into<PathBuf>,
) -> Router {
    router_with_runtime(
        profiles,
        Some(provider_settings),
        codex_program.into(),
        codex_home.into(),
    )
}

fn router_with_runtime(
    profiles: AgentProfiles,
    provider_settings: Option<ProviderSettingsStore>,
    codex_program: PathBuf,
    codex_home: PathBuf,
) -> Router {
    let provider_routes = Router::new()
        .route(
            "/v1/providers",
            get(list_provider_settings)
                .post(create_provider)
                .fallback(api_method_not_allowed),
        )
        .route(
            "/v1/providers/{kind}",
            axum::routing::put(update_provider)
                .delete(delete_provider)
                .fallback(api_method_not_allowed),
        )
        .layer(from_fn(require_loopback_provider_admin));

    Router::new()
        .route("/healthz", get(health))
        .route(
            "/v1/profiles",
            get(list_profiles).fallback(api_method_not_allowed),
        )
        .route(
            "/v1/respond",
            post(respond).fallback(api_method_not_allowed),
        )
        .route(
            "/v1/respond/stream",
            post(respond_stream).fallback(api_method_not_allowed),
        )
        .merge(provider_routes)
        .route("/v1", any(api_not_found))
        .route("/v1/{*path}", any(api_not_found))
        .with_state(AppState {
            profiles: Arc::new(profiles),
            provider_settings: provider_settings.map(|store| Arc::new(Mutex::new(store))),
            codex_program,
            codex_home,
        })
}

async fn require_loopback_provider_admin(
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let is_loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(address)| address.ip().is_loopback());
    if is_loopback {
        next.run(request).await
    } else {
        ApiError {
            status: StatusCode::FORBIDDEN,
            code: "provider_admin_forbidden",
            message: "provider administration is restricted to loopback clients".to_owned(),
        }
        .into_response()
    }
}

pub fn router_with_web(agent: Agent, web_dir: impl AsRef<Path>) -> Router {
    let profile = Profile {
        name: "default".to_owned(),
        provider: "configured".to_owned(),
        model: "configured".to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
        capabilities: Vec::new(),
    };
    let profiles = AgentProfiles::new("default", [(profile, agent)])
        .expect("the built-in server profile must be valid");
    router_with_profiles_and_web(profiles, web_dir)
}

pub fn router_with_profiles_and_web(profiles: AgentProfiles, web_dir: impl AsRef<Path>) -> Router {
    let web_dir = web_dir.as_ref();
    let index = web_dir.join("index.html");
    let assets = ServeDir::new(web_dir).fallback(ServeFile::new(index));
    router_with_profiles(profiles).fallback_service(assets)
}

pub fn router_with_profiles_provider_settings_and_web(
    profiles: AgentProfiles,
    provider_settings: ProviderSettingsStore,
    web_dir: impl AsRef<Path>,
) -> Router {
    let web_dir = web_dir.as_ref();
    let index = web_dir.join("index.html");
    let assets = ServeDir::new(web_dir).fallback(ServeFile::new(index));
    router_with_profiles_and_provider_settings(profiles, provider_settings).fallback_service(assets)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Serialize)]
struct ProfilesResponse {
    default_profile: String,
    profiles: Vec<Profile>,
}

async fn list_profiles(State(state): State<AppState>) -> Json<ProfilesResponse> {
    Json(ProfilesResponse {
        default_profile: state.profiles.default_profile().to_owned(),
        profiles: state.profiles.profiles(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProviderInput {
    Ollama {
        api_base: String,
    },
    #[serde(rename = "openai")]
    OpenAi {
        authentication: OpenAiAuthentication,
        #[serde(default)]
        api_key: Option<String>,
    },
}

impl ProviderInput {
    fn kind(&self) -> &'static str {
        match self {
            Self::Ollama { .. } => "ollama",
            Self::OpenAi { .. } => "openai",
        }
    }
}

fn provider_store(state: &AppState) -> Result<&Arc<Mutex<ProviderSettingsStore>>, ApiError> {
    state.provider_settings.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "provider_settings_unavailable",
        message: "provider settings are unavailable".to_owned(),
    })
}

async fn list_provider_settings(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConfiguredProvider>>, ApiError> {
    let mut store = provider_store(&state)?.lock().await;
    store.refresh().map_err(provider_store_error)?;
    Ok(Json(store.list()))
}

async fn create_provider(
    State(state): State<AppState>,
    request: Result<Json<ProviderInput>, JsonRejection>,
) -> Result<Json<ConfiguredProvider>, ApiError> {
    let Json(input) = request.map_err(ApiError::from)?;
    let mut store = provider_store(&state)?.lock().await;
    store.refresh().map_err(provider_store_error)?;
    if store.get(input.kind()).is_some() {
        return Err(provider_settings_error(format!(
            "provider `{}` is already configured",
            input.kind()
        )));
    }
    let provider = configured_provider(&state, input).await?;
    store.add(provider.clone()).map_err(provider_store_error)?;
    Ok(Json(provider))
}

async fn update_provider(
    State(state): State<AppState>,
    AxumPath(kind): AxumPath<String>,
    request: Result<Json<ProviderInput>, JsonRejection>,
) -> Result<Json<ConfiguredProvider>, ApiError> {
    let Json(input) = request.map_err(ApiError::from)?;
    if kind != input.kind() {
        return Err(provider_settings_error(
            "provider path and request kind must match",
        ));
    }
    let mut store = provider_store(&state)?.lock().await;
    store.refresh().map_err(provider_store_error)?;
    if store.get(&kind).is_none() {
        return Err(provider_settings_error(format!(
            "provider `{kind}` is not configured"
        )));
    }
    let provider = configured_provider(&state, input).await?;
    store
        .update(provider.clone())
        .map_err(provider_store_error)?;
    Ok(Json(provider))
}

async fn delete_provider(
    State(state): State<AppState>,
    AxumPath(kind): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    provider_store(&state)?
        .lock()
        .await
        .delete(&kind)
        .map_err(provider_store_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn configured_provider(
    state: &AppState,
    input: ProviderInput,
) -> Result<ConfiguredProvider, ApiError> {
    match input {
        ProviderInput::Ollama { api_base } => Ok(ConfiguredProvider::Ollama { api_base }),
        ProviderInput::OpenAi {
            authentication,
            api_key,
        } => {
            authenticate_openai(state, authentication, api_key).await?;
            Ok(ConfiguredProvider::OpenAi { authentication })
        }
    }
}

async fn authenticate_openai(
    state: &AppState,
    authentication: OpenAiAuthentication,
    api_key: Option<String>,
) -> Result<(), ApiError> {
    secure_private_directory(state.codex_home.clone())
        .map_err(|_| provider_settings_error("failed to prepare secure OpenAI credentials"))?;
    let mut command = Command::new(&state.codex_program);
    command
        .env("CODEX_HOME", &state.codex_home)
        .arg("login")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match authentication {
        OpenAiAuthentication::Chatgpt => command
            .stdin(Stdio::null())
            .spawn()
            .map_err(|_| provider_settings_error("failed to start ChatGPT sign-in"))?,
        OpenAiAuthentication::ApiKey => {
            let api_key = api_key
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| provider_settings_error("OpenAI API key must not be blank"))?;
            if api_key.len() > 16 * 1024 {
                return Err(provider_settings_error("OpenAI API key is too large"));
            }
            let mut child = command
                .arg("--with-api-key")
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|_| provider_settings_error("failed to start OpenAI API-key sign-in"))?;
            let mut stdin = child.stdin.take().ok_or_else(|| {
                provider_settings_error("OpenAI API-key sign-in input is unavailable")
            })?;
            stdin
                .write_all(api_key.as_bytes())
                .await
                .map_err(|_| provider_settings_error("failed to send the OpenAI API key"))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|_| provider_settings_error("failed to send the OpenAI API key"))?;
            drop(stdin);
            child
        }
    };
    let wait = match authentication {
        OpenAiAuthentication::Chatgpt => Duration::from_secs(300),
        OpenAiAuthentication::ApiKey => Duration::from_secs(120),
    };
    let status = timeout(wait, child.wait())
        .await
        .map_err(|_| provider_settings_error("OpenAI sign-in timed out"))?
        .map_err(|_| provider_settings_error("failed to wait for OpenAI sign-in"))?;
    if !status.success() {
        return Err(provider_settings_error("OpenAI sign-in did not complete"));
    }
    Ok(())
}

fn provider_settings_error(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "invalid_provider_settings",
        message: message.into(),
    }
}

fn provider_store_error(error: ProviderSettingsError) -> ApiError {
    match error {
        ProviderSettingsError::Duplicate(_)
        | ProviderSettingsError::NotConfigured(_)
        | ProviderSettingsError::InvalidOllamaUrl(_)
        | ProviderSettingsError::UnsafeOllamaUrl => provider_settings_error(error.to_string()),
        _ => ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "provider_settings_persistence_failed",
            message: "provider settings could not be persisted".to_owned(),
        },
    }
}

async fn api_not_found() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "API route not found".to_owned(),
    }
}

async fn api_method_not_allowed() -> ApiError {
    ApiError {
        status: StatusCode::METHOD_NOT_ALLOWED,
        code: "method_not_allowed",
        message: "HTTP method is not allowed for this API route".to_owned(),
    }
}

#[derive(Deserialize)]
pub struct RespondRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub history: Vec<Message>,
}

#[derive(Serialize)]
pub struct RespondResponse {
    pub message: Message,
}

async fn respond(
    State(state): State<AppState>,
    request: Result<Json<RespondRequest>, JsonRejection>,
) -> Result<Json<RespondResponse>, ApiError> {
    let Json(request) = request.map_err(ApiError::from)?;
    let message = state
        .profiles
        .respond(
            request.profile.as_deref(),
            &request.history,
            &request.prompt,
        )
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RespondResponse { message }))
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StreamResponseEvent {
    Thinking { content: String },
    Content { content: String },
    Done { message: Message },
    Error { message: String },
}

impl From<&CompletionDelta> for StreamResponseEvent {
    fn from(delta: &CompletionDelta) -> Self {
        match delta {
            CompletionDelta::Thinking(content) => Self::Thinking {
                content: content.clone(),
            },
            CompletionDelta::Content(content) => Self::Content {
                content: content.clone(),
            },
        }
    }
}

async fn respond_stream(
    State(state): State<AppState>,
    request: Result<Json<RespondRequest>, JsonRejection>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let Json(request) = request.map_err(ApiError::from)?;
    let profiles = Arc::clone(&state.profiles);
    let (sender, receiver) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let delta_sender = sender.clone();
        let mut on_delta = move |delta: &CompletionDelta| {
            let _ = delta_sender.send(StreamResponseEvent::from(delta));
        };
        let result = profiles
            .respond_stream(
                request.profile.as_deref(),
                &request.history,
                &request.prompt,
                &mut on_delta,
            )
            .await;
        let event = match result {
            Ok(message) => StreamResponseEvent::Done { message },
            Err(error) => StreamResponseEvent::Error {
                message: ApiError::from(error).message,
            },
        };
        let _ = sender.send(event);
    });

    let stream = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| {
            let event = Event::default()
                .json_data(event)
                .expect("stream response events must serialize");
            (Ok(event), receiver)
        })
    });
    Ok(Sse::new(stream))
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl From<AgentError> for ApiError {
    fn from(error: AgentError) -> Self {
        match error {
            AgentError::BlankInput | AgentError::InvalidHistory => Self {
                status: StatusCode::BAD_REQUEST,
                code: "invalid_request",
                message: error.to_string(),
            },
            AgentError::Provider(_)
            | AgentError::InvalidProviderResponse
            | AgentError::EmptyProviderResponse
            | AgentError::UnexpectedToolCallAfterFinalAnswer => Self {
                status: StatusCode::BAD_GATEWAY,
                code: "provider_error",
                message: "model provider request failed".to_owned(),
            },
            AgentError::ToolLoopLimit(_)
            | AgentError::ToolCallLimit(_)
            | AgentError::ToolResultByteLimit(_)
            | AgentError::ToolExecutionDeadline(_)
            | AgentError::ToolLoopDeadline(_) => Self {
                status: StatusCode::BAD_GATEWAY,
                code: "tool_loop_error",
                message: error.to_string(),
            },
            AgentError::BlankToolName | AgentError::DuplicateTool(_) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "agent_configuration_error",
                message: "agent tool configuration is invalid".to_owned(),
            },
        }
    }
}

impl From<ProfileAgentError> for ApiError {
    fn from(error: ProfileAgentError) -> Self {
        match error {
            ProfileAgentError::UnknownProfile(profile) => Self {
                status: StatusCode::BAD_REQUEST,
                code: "unknown_profile",
                message: format!("profile `{profile}` is not defined"),
            },
            ProfileAgentError::Agent(error) => Self::from(error),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(error: JsonRejection) -> Self {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                code: "payload_too_large",
                message: "request body is too large".to_owned(),
            };
        }

        if error.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE {
            return Self {
                status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
                code: "unsupported_media_type",
                message: "content type must be application/json".to_owned(),
            };
        }

        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: "request body must be valid JSON".to_owned(),
        }
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}
