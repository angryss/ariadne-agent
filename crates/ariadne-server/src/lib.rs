use std::path::Path;
use std::sync::Arc;

use ariadne_core::{Agent, AgentError, AgentProfiles, Message, Profile, ProfileAgentError};
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    profiles: Arc<AgentProfiles>,
}

pub fn router(agent: Agent) -> Router {
    let profile = Profile {
        name: "default".to_owned(),
        provider: "configured".to_owned(),
        model: "configured".to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let profiles = AgentProfiles::new("default", [(profile, agent)])
        .expect("the built-in server profile must be valid");
    router_with_profiles(profiles)
}

pub fn router_with_profiles(profiles: AgentProfiles) -> Router {
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
        .route("/v1", any(api_not_found))
        .route("/v1/{*path}", any(api_not_found))
        .with_state(AppState {
            profiles: Arc::new(profiles),
        })
}

pub fn router_with_web(agent: Agent, web_dir: impl AsRef<Path>) -> Router {
    let profile = Profile {
        name: "default".to_owned(),
        provider: "configured".to_owned(),
        model: "configured".to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
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
            AgentError::Provider(_) | AgentError::InvalidProviderResponse => Self {
                status: StatusCode::BAD_GATEWAY,
                code: "provider_error",
                message: "model provider request failed".to_owned(),
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
