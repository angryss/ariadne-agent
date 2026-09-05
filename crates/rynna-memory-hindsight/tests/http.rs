use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::{get, post},
};
use rynna_config::memory::{HindsightDeployment, MemorySettings};
use rynna_core::{MemoryConversation, MemoryMessage, MemoryProvider, Role};
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
        memory
            .retain(&conversation(uuid::Uuid::nil()))
            .await
            .unwrap();
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
        let body = &records[1].2;
        assert_eq!(body["async"], true);
        let item = &body["items"][0];
        assert!(
            item["document_id"]
                .as_str()
                .unwrap()
                .starts_with("rynna-00000000-0000-0000-0000-000000000000-")
        );
        assert!(
            body.get("document_id").is_none(),
            "the SDK maps document_id onto each HTTP item"
        );
        assert!(item.get("update_mode").is_none());
        let content: Value = serde_json::from_str(item["content"].as_str().unwrap()).unwrap();
        assert_eq!(content[0]["role"], "user");
        assert_eq!(content[0]["content"], "question");
        assert_eq!(content[1]["role"], "assistant");
        assert_eq!(content[1]["content"], "answer");
        assert_eq!(content[0]["timestamp"], "2026-09-05T01:00:00.000Z");
        assert_eq!(item["metadata"]["source"], "rynna");
        assert_eq!(records[1].1, key.map(|key| format!("Bearer {key}")));
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

fn conversation(session_id: uuid::Uuid) -> MemoryConversation {
    let timestamp = "2026-09-05T01:00:00.000Z".to_owned();
    MemoryConversation {
        session_id,
        timestamp: timestamp.clone(),
        messages: [(Role::User, "question"), (Role::Assistant, "answer")]
            .into_iter()
            .map(|(role, content)| MemoryMessage {
                role,
                content: content.into(),
                timestamp: Some(timestamp.clone()),
            })
            .collect(),
    }
}

#[tokio::test]
async fn modern_servers_append_only_new_turns_and_legacy_servers_replace_complete_snapshots() {
    for version in ["0.5.0", "0.4.9", "unrecognized"] {
        let requests = Requests::default();
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = probes.clone();
        let app = Router::new()
            .route(
                "/proxy/version",
                get(move |headers: HeaderMap| {
                    let count = count.clone();
                    async move {
                        assert_eq!(headers["authorization"], "Bearer test-secret");
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Json(json!({"version":version}))
                    }
                }),
            )
            .route("/{*path}", post(record))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/proxy/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let config = settings(base, Some("test-secret"));
        let memory = HindsightMemoryProvider::new(&config).unwrap();
        let mut transcript = conversation(uuid::Uuid::new_v4());
        memory.retain(&transcript).await.unwrap();
        transcript
            .messages
            .extend(conversation(transcript.session_id).messages);
        transcript.messages[2].content = "follow-up".into();
        memory.retain(&transcript).await.unwrap();
        let records = requests.lock().unwrap().clone();
        let first = &records[0].2["items"][0];
        let second = &records[1].2["items"][0];
        assert_eq!(first["document_id"], second["document_id"]);
        let content: Vec<Value> =
            serde_json::from_str(second["content"].as_str().unwrap()).unwrap();
        if version == "0.5.0" {
            assert_eq!(
                second["document_id"],
                format!("rynna-{}", transcript.session_id)
            );
            assert_eq!(second["update_mode"], "append");
            assert_eq!(content.len(), 2);
            assert_eq!(content[0]["content"], "follow-up");
        } else {
            assert!(second.get("update_mode").is_none());
            assert_eq!(content.len(), 4);
        }
        assert_eq!(second["metadata"]["turn_index"], "2");
        assert_eq!(
            second["tags"],
            json!([format!("session:{}", transcript.session_id)])
        );
        assert_eq!(probes.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Restart uses the same append document, but never overwrites a legacy process's document.
        HindsightMemoryProvider::new(&config)
            .unwrap()
            .retain(&transcript)
            .await
            .unwrap();
        let restarted = requests.lock().unwrap()[2].2["items"][0]["document_id"].clone();
        assert_eq!(restarted == second["document_id"], version == "0.5.0");
        memory
            .retain(&conversation(uuid::Uuid::new_v4()))
            .await
            .unwrap();
        assert_ne!(
            requests.lock().unwrap()[3].2["items"][0]["document_id"],
            second["document_id"]
        );
        server.abort();
    }
}
