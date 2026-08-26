use std::sync::Arc;

use ariadne_core::{
    Agent, AgentProfiles, Completion, CompletionDelta, CompletionRequest, Message, ModelProvider,
    Profile, ProviderError,
};
use ariadne_server::{router, router_with_profiles, router_with_web};
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

struct FixedProvider;

#[async_trait]
impl ModelProvider for FixedProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant("Ready.")))
    }
}

struct InvalidRoleProvider;

#[async_trait]
impl ModelProvider for InvalidRoleProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::user("not an assistant response")))
    }
}

struct ReplyProvider(&'static str);

#[async_trait]
impl ModelProvider for ReplyProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant(self.0)))
    }
}

struct ThinkingProvider;

#[async_trait]
impl ModelProvider for ThinkingProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant("Answer")))
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Completion, ProviderError> {
        on_delta(&CompletionDelta::Thinking("Inspect".to_owned()));
        on_delta(&CompletionDelta::Content("Answer".to_owned()));
        Ok(Completion::new(Message::assistant("Answer")))
    }
}

fn test_app() -> axum::Router {
    let agent = ariadne_core::Agent::new(Arc::new(FixedProvider), "You are Ariadne.");
    router(agent)
}

fn profile(name: &str, reply: &'static str) -> (Profile, Agent) {
    (
        Profile {
            name: name.to_owned(),
            provider: format!("{name}-provider"),
            model: format!("{name}-model"),
            active_skills: vec![format!("{name}-skill")],
            mcp_servers: vec![format!("{name}-mcp")],
        },
        Agent::new(Arc::new(ReplyProvider(reply)), "You are Ariadne."),
    )
}

fn profiles_app() -> axum::Router {
    router_with_profiles(
        AgentProfiles::new(
            "local",
            vec![profile("local", "Local."), profile("work", "Work.")],
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn health_endpoint_reports_the_service_is_ready() {
    let response = test_app()
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value, serde_json::json!({"status": "ok"}));
}

#[tokio::test]
async fn respond_endpoint_returns_an_assistant_message() {
    let response = test_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "prompt": "Hello",
                        "history": [{"role": "user", "content": "Earlier"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "message": {"role": "assistant", "content": "Ready."}
        })
    );
}

#[tokio::test]
async fn respond_stream_endpoint_emits_reasoning_content_and_completion_events() {
    let agent = Agent::new(Arc::new(ThinkingProvider), "You are Ariadne.");
    let response = router(agent)
        .oneshot(
            Request::post("/v1/respond/stream")
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .body(Body::from(r#"{"prompt":"Hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        body.contains(r#"data: {"kind":"thinking","content":"Inspect"}"#),
        "{body}"
    );
    assert!(
        body.contains(r#"data: {"kind":"content","content":"Answer"}"#),
        "{body}"
    );
    assert!(
        body.contains(r#"data: {"kind":"done","message":{"role":"assistant","content":"Answer"}}"#),
        "{body}"
    );
}

#[tokio::test]
async fn profiles_endpoint_lists_profiles_and_respond_dispatches_to_the_requested_profile() {
    let profiles_response = profiles_app()
        .oneshot(Request::get("/v1/profiles").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(profiles_response.status(), StatusCode::OK);
    let profiles_body = to_bytes(profiles_response.into_body(), 4096).await.unwrap();
    let profiles: Value = serde_json::from_slice(&profiles_body).unwrap();
    assert_eq!(profiles["default_profile"], "local");
    assert_eq!(profiles["profiles"][1]["name"], "work");
    assert_eq!(profiles["profiles"][1]["active_skills"][0], "work-skill");
    assert_eq!(profiles["profiles"][1]["mcp_servers"][0], "work-mcp");

    let response = profiles_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"profile":"work","prompt":"Hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(value["message"]["content"], "Work.");
}

#[tokio::test]
async fn respond_endpoint_rejects_an_unknown_profile() {
    let response = profiles_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"profile":"missing","prompt":"Hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "unknown_profile",
                "message": "profile `missing` is not defined"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_returns_json_for_malformed_json() {
    let response = test_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"prompt":}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "invalid_request",
                "message": "request body must be valid JSON"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_requires_a_json_content_type() {
    let response = test_app()
        .oneshot(
            Request::post("/v1/respond")
                .body(Body::from(r#"{"prompt":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "unsupported_media_type",
                "message": "content type must be application/json"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_returns_json_for_an_oversized_body() {
    let oversized_body = serde_json::json!({
        "prompt": "x".repeat(2 * 1024 * 1024)
    })
    .to_string();
    let response = test_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(oversized_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "payload_too_large",
                "message": "request body is too large"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_returns_json_when_the_method_is_not_allowed() {
    let response = test_app()
        .oneshot(Request::get("/v1/respond").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "method_not_allowed",
                "message": "HTTP method is not allowed for this API route"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_rejects_a_blank_prompt_as_bad_input() {
    let response = test_app()
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"prompt":"   "}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {
                "code": "invalid_request",
                "message": "user input must not be blank"
            }
        })
    );
}

#[tokio::test]
async fn respond_endpoint_hides_invalid_provider_responses() {
    let agent = ariadne_core::Agent::new(Arc::new(InvalidRoleProvider), "You are Ariadne.");
    let response = router(agent)
        .oneshot(
            Request::post("/v1/respond")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"prompt":"hello","history":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {"code": "provider_error", "message": "model provider request failed"}
        })
    );
}

#[tokio::test]
async fn web_router_serves_the_spa_for_browser_routes() {
    let web_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        web_dir.path().join("index.html"),
        "<!doctype html><title>Ariadne</title>",
    )
    .unwrap();
    let agent = ariadne_core::Agent::new(Arc::new(FixedProvider), "You are Ariadne.");

    let response = router_with_web(agent, web_dir.path())
        .oneshot(
            Request::get("/conversations/new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(
        String::from_utf8(body.to_vec())
            .unwrap()
            .contains("Ariadne")
    );
}

#[tokio::test]
async fn web_router_does_not_hide_unknown_api_routes_behind_the_spa() {
    let web_dir = tempfile::tempdir().unwrap();
    std::fs::write(web_dir.path().join("index.html"), "<title>Ariadne</title>").unwrap();
    let agent = ariadne_core::Agent::new(Arc::new(FixedProvider), "You are Ariadne.");

    let response = router_with_web(agent, web_dir.path())
        .oneshot(Request::get("/v1/missing").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "error": {"code": "not_found", "message": "API route not found"}
        })
    );
}
