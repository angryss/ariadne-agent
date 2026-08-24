use std::sync::Arc;

use ariadne_core::{Completion, CompletionRequest, Message, ModelProvider, ProviderError};
use ariadne_server::{router, router_with_web};
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

fn test_app() -> axum::Router {
    let agent = ariadne_core::Agent::new(Arc::new(FixedProvider), "You are Ariadne.");
    router(agent)
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
