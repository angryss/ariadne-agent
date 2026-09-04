use async_trait::async_trait;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{Request, StatusCode, Uri},
    routing::post,
};
use rynna_config::ProviderSettingsStore;
use rynna_core::{
    Agent, AgentProfiles, Completion, CompletionRequest, Message, ModelProvider, Profile,
    ProviderError,
};
use rynna_server::router_with_profiles_and_provider_settings;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

#[derive(Default)]
struct Model(Mutex<Vec<CompletionRequest>>);
#[async_trait]
impl ModelProvider for Model {
    async fn complete(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        self.0.lock().unwrap().push(request);
        Ok(Completion::new(Message::assistant("answer")))
    }
}
fn app(path: &std::path::Path, model: Arc<Model>) -> Router {
    let profile = Profile {
        name: "test".into(),
        providers: vec![],
        active_skills: vec![],
        mcp_servers: vec![],
        capabilities: vec![],
    };
    let profiles = AgentProfiles::new("test", [(profile, Agent::new(model, "policy"))]).unwrap();
    router_with_profiles_and_provider_settings(profiles, ProviderSettingsStore::load(path).unwrap())
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    value: Value,
    local: bool,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap();
    if local {
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()));
    }
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn settings_are_local_only_default_to_none_and_never_return_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("providers.toml");
    let app = app(&path, Arc::new(Model::default()));
    for method in ["GET", "PUT"] {
        assert_eq!(
            request(
                &app,
                method,
                "/v1/settings/memory",
                json!({"kind":"none"}),
                false
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(&app, "GET", "/v1/settings/memory", Value::Null, true)
            .await
            .1,
        json!({"kind":"none"})
    );
    let input = json!({"kind":"hindsight", "deployment":"cloud", "api_base":"https://api.hindsight.vectorize.io", "bank_id":"rynna", "api_key":"test-secret"});
    let (status, saved) = request(&app, "PUT", "/v1/settings/memory", input, true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["api_key_configured"], true);
    assert!(!saved.to_string().contains("test-secret"));
    assert!(saved.get("api_key").is_none());
    assert_eq!(
        request(&app, "GET", "/v1/settings/memory", Value::Null, true)
            .await
            .1,
        saved
    );
    let mut malformed = saved.clone();
    malformed["deployment"] = json!("test-secret");
    let (status, body) = request(&app, "PUT", "/v1/settings/memory", malformed, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.to_string().contains("test-secret"));
    assert_eq!(
        request(&app, "GET", "/v1/settings/memory", Value::Null, true)
            .await
            .1,
        saved
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            "/v1/settings/memory",
            json!({"kind":"none"}),
            true
        )
        .await
        .1,
        json!({"kind":"none"})
    );
}

type Calls = Arc<Mutex<Vec<String>>>;
async fn memory(State(calls): State<Calls>, uri: Uri, Json(_): Json<Value>) -> Json<Value> {
    calls.lock().unwrap().push(uri.path().into());
    Json(json!({"results":[{"text":"Remembered fact"}],"success":true}))
}
#[tokio::test]
async fn saving_and_disabling_change_live_sync_and_stream_requests_and_survive_restart() {
    let calls = Calls::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let memory_app = Router::new()
        .route("/{*path}", post(memory))
        .with_state(calls.clone());
    let server = tokio::spawn(async move { axum::serve(listener, memory_app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("providers.toml");
    let model = Arc::new(Model::default());
    let router = app(&path, model.clone());
    let settings =
        json!({"kind":"hindsight","deployment":"self_hosted","api_base":base,"bank_id":"test"});
    assert_eq!(
        request(&router, "PUT", "/v1/settings/memory", settings, true)
            .await
            .0,
        StatusCode::OK
    );
    let prompt = json!({"prompt":"hello","history":[]});
    assert_eq!(
        request(&router, "POST", "/v1/respond", prompt.clone(), true)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        model.0.lock().unwrap()[0].messages[0]
            .content
            .contains("Remembered fact")
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    let restarted = app(&path, model.clone());
    let stream = Request::post("/v1/respond/stream")
        .header("content-type", "application/json")
        .body(Body::from(prompt.to_string()))
        .unwrap();
    let response = restarted.oneshot(stream).await.unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("answer"));
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert_eq!(
        request(
            &router,
            "PUT",
            "/v1/settings/memory",
            json!({"kind":"none"}),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    request(&router, "POST", "/v1/respond", prompt, true).await;
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert_eq!(
        model.0.lock().unwrap().last().unwrap().messages[0].content,
        "policy"
    );
    server.abort();
}

#[tokio::test]
async fn failed_persistence_is_reported_and_does_not_enable_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("providers.toml");
    let router = app(&path, Arc::new(Model::default()));
    std::fs::create_dir(dir.path().join("memory.toml")).unwrap();
    assert_eq!(
        request(
            &router,
            "PUT",
            "/v1/settings/memory",
            json!({"kind":"none"}),
            true
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
