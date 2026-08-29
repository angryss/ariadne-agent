use std::sync::Arc;

use ariadne_core::{
    CacheOptimization, CacheOptimizer, CompletionDelta, CompletionRequest, Message, ModelProvider,
    ToolCall, ToolDefinition,
};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ariadne_provider_openai::OpenAiCompatibleProvider;

struct DisabledCacheOptimizer;

impl CacheOptimizer for DisabledCacheOptimizer {
    fn optimize(&self, _request: &CompletionRequest) -> CacheOptimization {
        CacheOptimization {
            use_server_cache: false,
            scope_key: "unused".to_owned(),
        }
    }
}

struct ArbitraryScopeCacheOptimizer;

impl CacheOptimizer for ArbitraryScopeCacheOptimizer {
    fn optimize(&self, _request: &CompletionRequest) -> CacheOptimization {
        CacheOptimization {
            use_server_cache: true,
            scope_key: "x".repeat(200),
        }
    }
}

#[test]
fn remote_http_endpoint_rejects_api_key() {
    let secret = "super-secret";
    let error = OpenAiCompatibleProvider::new(
        "http://api.example.com/v1",
        "test-model",
        Some(secret.to_owned()),
    )
    .err()
    .expect("remote HTTP endpoint with credentials must be rejected");

    assert!(error.to_string().contains("HTTPS"));
    assert!(!error.to_string().contains(secret));
}

#[test]
fn unsupported_url_schemes_are_rejected() {
    let error = OpenAiCompatibleProvider::new("ftp://api.example.com/v1", "test-model", None)
        .err()
        .expect("non-HTTP schemes must be rejected");

    assert!(error.to_string().contains("HTTP or HTTPS"));
}

#[test]
fn provider_urls_with_embedded_credentials_are_rejected() {
    let error = OpenAiCompatibleProvider::new(
        "http://user:password@api.example.com/v1",
        "test-model",
        None,
    )
    .err()
    .expect("URL-embedded credentials must be rejected");

    assert!(error.to_string().contains("embedded credentials"));
    assert!(!error.to_string().contains("password"));
}

#[test]
fn loopback_http_endpoints_accept_api_keys() {
    for base_url in [
        "http://localhost:11434/v1",
        "http://127.0.0.1:11434/v1",
        "http://[::1]:11434/v1",
    ] {
        assert!(
            OpenAiCompatibleProvider::new(base_url, "test-model", Some("test-key".to_owned()))
                .is_ok(),
            "{base_url} should be allowed"
        );
    }
}

#[tokio::test]
async fn ollama_requests_use_the_cache_friendly_openai_compatible_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are Ariadne."},
                {"role": "user", "content": "Hello"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Hello from the model"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new_ollama(
        format!("{}/v1/", server.uri()),
        "test-model",
        Some("test-key".to_owned()),
    )
    .unwrap();

    let completion = provider
        .complete(CompletionRequest {
            messages: vec![Message::system("You are Ariadne."), Message::user("Hello")],
            tools: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(
        completion.message,
        Message::assistant("Hello from the model")
    );
}

#[tokio::test]
async fn openai_requests_include_a_stable_prompt_cache_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Hello"}
            }]
        })))
        .expect(2)
        .mount(&server)
        .await;
    let provider = OpenAiCompatibleProvider::new_openai(
        format!("{}/v1", server.uri()),
        "test-model",
        Some("test-key".to_owned()),
    )
    .unwrap();

    for messages in [
        vec![Message::system("Stable policy"), Message::user("First")],
        vec![
            Message::system("Stable policy"),
            Message::user("First"),
            Message::assistant("Answer"),
            Message::user("Second"),
        ],
    ] {
        provider
            .complete(CompletionRequest {
                messages,
                tools: Vec::new(),
            })
            .await
            .unwrap();
    }

    let requests = server.received_requests().await.unwrap();
    let bodies = requests
        .iter()
        .map(|request| serde_json::from_slice::<serde_json::Value>(&request.body).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(bodies[0]["prompt_cache_key"], bodies[1]["prompt_cache_key"]);
    assert_eq!(bodies[0]["prompt_cache_key"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn openai_cache_optimizer_can_be_substituted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "Stable policy"},
                {"role": "user", "content": "Hello"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Done"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new_openai(format!("{}/v1", server.uri()), "test-model", None)
            .unwrap()
            .with_cache_optimizer(Arc::new(DisabledCacheOptimizer));

    provider
        .complete(CompletionRequest {
            messages: vec![Message::system("Stable policy"), Message::user("Hello")],
            tools: Vec::new(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn openai_normalizes_substituted_cache_scopes_for_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Done"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new_openai(format!("{}/v1", server.uri()), "test-model", None)
            .unwrap()
            .with_cache_optimizer(Arc::new(ArbitraryScopeCacheOptimizer));

    provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: Vec::new(),
        })
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let cache_key = body["prompt_cache_key"].as_str().unwrap();
    assert_eq!(cache_key.len(), 64);
    assert_ne!(cache_key, "x".repeat(200));
}

#[tokio::test]
async fn complete_sends_tool_definitions_and_parses_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "Read README.md"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "required": ["path"]
                    }
                }
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        OpenAiCompatibleProvider::new(format!("{}/v1", server.uri()), "test-model", None).unwrap();

    let completion = provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Read README.md")],
            tools: vec![ToolDefinition::new(
                "read_file",
                "Read a workspace file",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            )],
        })
        .await
        .unwrap();

    assert_eq!(completion.message.tool_calls.len(), 1);
    assert_eq!(completion.message.tool_calls[0].name, "read_file");
    assert_eq!(
        completion.message.tool_calls[0].arguments,
        json!({"path": "README.md"})
    );
}

#[tokio::test]
async fn complete_serializes_assistant_tool_calls_and_tool_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "user", "content": "Read it"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                },
                {"role": "tool", "content": "{\"content\":\"# Ariadne\"}", "tool_call_id": "call-1"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content": "Done"}}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        OpenAiCompatibleProvider::new(format!("{}/v1", server.uri()), "test-model", None).unwrap();

    provider
        .complete(CompletionRequest {
            messages: vec![
                Message::user("Read it"),
                Message::assistant_with_tool_calls(vec![ToolCall::new(
                    "call-1",
                    "read_file",
                    json!({"path": "README.md"}),
                )]),
                Message::tool("call-1", "{\"content\":\"# Ariadne\"}"),
            ],
            tools: Vec::new(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn complete_stream_distinguishes_reasoning_from_user_facing_content() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": true
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"Check\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"reasoning\":\" facts\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
                    "data: [DONE]\n\n"
                )),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        OpenAiCompatibleProvider::new(format!("{}/v1", server.uri()), "test-model", None).unwrap();
    let mut deltas = Vec::new();
    let mut on_delta = |delta: &CompletionDelta| deltas.push(delta.clone());

    let completion = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: Vec::new(),
            },
            &mut on_delta,
        )
        .await
        .unwrap();

    assert_eq!(
        deltas,
        [
            CompletionDelta::Thinking("Check".to_owned()),
            CompletionDelta::Thinking(" facts".to_owned()),
            CompletionDelta::Content("Hello".to_owned()),
            CompletionDelta::Content(" world".to_owned()),
        ]
    );
    assert_eq!(completion.message, Message::assistant("Hello world"));
}

#[tokio::test]
async fn complete_stream_accumulates_fragmented_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"README.md\\\"}\"}}]}}]}\n\n",
                    "data: [DONE]\n\n"
                )),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        OpenAiCompatibleProvider::new(format!("{}/v1", server.uri()), "test-model", None).unwrap();
    let mut deltas = Vec::new();

    let completion = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Read README.md")],
                tools: vec![ToolDefinition::new(
                    "read_file",
                    "Read a workspace file",
                    json!({"type": "object"}),
                )],
            },
            &mut |delta| deltas.push(delta.clone()),
        )
        .await
        .unwrap();

    assert!(deltas.is_empty());
    assert_eq!(completion.message.tool_calls.len(), 1);
    assert_eq!(completion.message.tool_calls[0].id, "call-1");
    assert_eq!(
        completion.message.tool_calls[0].arguments,
        json!({"path": "README.md"})
    );
}

async fn incomplete_stream_error(body: &str) -> String {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiCompatibleProvider::new(server.uri(), "test-model", None).unwrap();

    provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: Vec::new(),
            },
            &mut |_| {},
        )
        .await
        .expect_err("an SSE response without a protocol terminator must fail")
        .to_string()
}

#[tokio::test]
async fn complete_stream_rejects_abrupt_empty_success_response() {
    let error = incomplete_stream_error("").await;
    assert!(error.contains("before [DONE]"), "unexpected error: {error}");
}

#[tokio::test]
async fn complete_stream_rejects_truncated_answer() {
    let error = incomplete_stream_error(
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial answer\"}}]}\n\n",
    )
    .await;
    assert!(error.contains("before [DONE]"), "unexpected error: {error}");
}

#[tokio::test]
async fn complete_stream_rejects_complete_looking_tool_call_without_terminator() {
    let error = incomplete_stream_error(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{}\"}}]}}]}\n\n",
    )
    .await;
    assert!(error.contains("before [DONE]"), "unexpected error: {error}");
}

#[tokio::test]
async fn oversized_success_response_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 1024 * 1024 + 1]))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(server.uri(), "test-model", None).unwrap();
    let error = provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: Vec::new(),
        })
        .await
        .expect_err("oversized success body must be rejected");

    assert!(
        error.to_string().contains("1048576-byte limit"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn oversized_error_response_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_bytes(vec![b'x'; 1024 * 1024 + 1]))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(server.uri(), "test-model", None).unwrap();
    let error = provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: Vec::new(),
        })
        .await
        .expect_err("oversized error body must be rejected");

    assert!(
        error.to_string().contains("1048576-byte limit"),
        "unexpected error: {error}"
    );
}
