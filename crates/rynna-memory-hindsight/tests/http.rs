use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::post,
};
use rynna_config::memory::{HindsightDeployment, MemorySettings};
use rynna_core::MemoryProvider;
use rynna_memory_hindsight::HindsightMemoryProvider;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type Requests = Arc<Mutex<Vec<(String, Option<String>, Value)>>>;
async fn record(
    State(requests): State<Requests>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    requests.lock().unwrap().push((
        uri.path().into(),
        headers
            .get("authorization")
            .map(|header| header.to_str().unwrap().into()),
        body,
    ));
    Json(if uri.path().ends_with("recall") {
        json!({"results":[{"text":"Prefers Rust"}]})
    } else {
        json!({"success":true})
    })
}
fn settings(base: String, key: Option<&str>) -> MemorySettings {
    MemorySettings::Hindsight {
        deployment: HindsightDeployment::SelfHosted,
        api_base: base,
        bank_id: "bank/with spaces".into(),
        api_key: key.map(str::to_owned),
    }
}

#[tokio::test]
async fn recall_and_retain_use_documented_http_contract_with_optional_bearer_auth() {
    for key in [None, Some("test-secret")] {
        let requests = Requests::default();
        let app = Router::new()
            .route("/{*path}", post(record))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/proxy/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let memory = HindsightMemoryProvider::new(&settings(base, key)).unwrap();
        assert_eq!(memory.recall("language?").await.unwrap(), ["Prefers Rust"]);
        memory.retain("question", "answer").await.unwrap();
        let records = requests.lock().unwrap();
        assert_eq!(
            records[0].0,
            "/proxy/v1/default/banks/bank%2Fwith%20spaces/memories/recall"
        );
        assert_eq!(records[0].1, key.map(|key| format!("Bearer {key}")));
        assert_eq!(
            records[0].2,
            json!({"query":"language?", "max_tokens":2048,"budget":"low"})
        );
        assert_eq!(
            records[1].2,
            json!({"items":[{"content":"User: question\nAssistant: answer","context":"Rynna conversation"}],"async":true})
        );
        server.abort();
    }
}

#[tokio::test]
async fn errors_are_sanitized_and_redirects_are_not_followed() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::TEMPORARY_REDIRECT] {
        let app = Router::new().route(
            "/{*path}",
            post(move || async move {
                (
                    status,
                    [("location", "http://127.0.0.1:1/secret")],
                    "test-secret private conversation",
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let error = HindsightMemoryProvider::new(&settings(base, Some("test-secret")))
            .unwrap()
            .recall("private conversation")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("Hindsight returned HTTP {}", status.as_u16())
        );
        server.abort();
    }
}
