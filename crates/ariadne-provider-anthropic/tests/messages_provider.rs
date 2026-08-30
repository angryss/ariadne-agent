use std::sync::Arc;

use ariadne_core::{
    CacheOptimization, CacheOptimizer, CompletionDelta, CompletionRequest, ContextPlan,
    ContextSize, Message, ModelProvider, ProviderContext, ServerCompaction, ToolCall,
    ToolDefinition,
};
use ariadne_provider_anthropic::AnthropicMessagesProvider;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct DisabledCacheOptimizer;

impl CacheOptimizer for DisabledCacheOptimizer {
    fn optimize(&self, _request: &CompletionRequest) -> CacheOptimization {
        CacheOptimization {
            use_server_cache: false,
            scope_key: "unused".to_owned(),
        }
    }
}

async fn malformed_stream_error(body: &str) -> String {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body.to_owned()),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap_err()
        .to_string()
}

#[test]
fn remote_http_endpoint_rejects_api_key_without_exposing_it() {
    let secret = "super-secret";
    let error = match AnthropicMessagesProvider::with_base_url(
        "http://api.anthropic.example",
        "claude-test",
        secret,
    ) {
        Ok(_) => panic!("remote HTTP credentials should be rejected"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("HTTPS"));
    assert!(!error.to_string().contains(secret));
}

#[test]
fn ipv6_loopback_http_is_allowed_for_local_development() {
    AnthropicMessagesProvider::with_base_url("http://[::1]:8080", "claude-test", "secret")
        .unwrap_or_else(|error| panic!("IPv6 loopback should be accepted: {error}"));
}

#[tokio::test]
async fn messages_api_uses_anthropic_headers_and_content_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_json(json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "cache_control": {"type": "ephemeral"},
            "system": "You are Ariadne.",
            "messages": [{"role":"user","content":[{"type":"text","text":"Hello"}]}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type":"message","role":"assistant","content":[{"type":"text","text":"Hello from Claude"}]
        })))
        .expect(1).mount(&server).await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let completion = provider
        .complete(CompletionRequest {
            messages: vec![Message::system("You are Ariadne."), Message::user("Hello")],
            tools: vec![],
        })
        .await
        .unwrap();
    assert_eq!(completion.message, Message::assistant("Hello from Claude"));
}

#[tokio::test]
async fn messages_api_enables_server_side_compaction_for_managed_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-beta", "compact-2026-01-12"))
        .and(body_json(json!({
            "model": "claude-opus-4-6",
            "max_tokens": 4096,
            "cache_control": {"type": "ephemeral"},
            "messages": [{"role":"user","content":[{"type":"text","text":"Continue"}]}],
            "context_management": {"edits": [{
                "type": "compact_20260112",
                "trigger": {"type": "input_tokens", "value": 50000},
                "instructions": "Summarize the transcript for continuity. Do not call tools."
            }]}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content":[
                {"type":"compaction","content":"Earlier work was summarized."},
                {"type":"text","text":"Compacted answer"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-opus-4-6", "test-key")
            .unwrap();

    assert_eq!(
        provider.server_compaction(),
        Some(ServerCompaction::Anthropic)
    );
    let completion = provider
        .complete_managed(ContextPlan {
            request: CompletionRequest {
                messages: vec![Message::user("Continue")],
                tools: vec![],
            },
            size: ContextSize {
                current_tokens: 50_000,
                max_tokens: 60_000,
            },
            server_compaction_threshold: Some(40_000),
            compacted: false,
        })
        .await
        .unwrap();

    assert_eq!(completion.message.content, "Compacted answer");
    assert_eq!(
        completion.message.provider_context,
        Some(ProviderContext::AnthropicCompaction(Some(
            "Earlier work was summarized.".to_owned()
        )))
    );
}

#[test]
fn unsupported_anthropic_models_use_local_compaction_fallback() {
    let provider = AnthropicMessagesProvider::with_base_url(
        "http://127.0.0.1:3000",
        "claude-sonnet-4-5",
        "test-key",
    )
    .unwrap();

    assert_eq!(provider.server_compaction(), None);
}

#[tokio::test]
async fn messages_api_round_trips_compaction_blocks_on_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [
                {"type": "compaction", "content": "Earlier work was summarized."},
                {"type": "text", "text": "Compacted answer"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let compacted = provider
        .complete_managed(ContextPlan {
            request: CompletionRequest {
                messages: vec![Message::user("Start")],
                tools: vec![],
            },
            size: ContextSize {
                current_tokens: 50_000,
                max_tokens: 60_000,
            },
            server_compaction_threshold: Some(40_000),
            compacted: false,
        })
        .await
        .unwrap();
    assert_eq!(
        compacted.message.provider_context,
        Some(ProviderContext::AnthropicCompaction(Some(
            "Earlier work was summarized.".to_owned()
        )))
    );

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-beta", "compact-2026-01-12"))
        .and(body_json(json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "cache_control": {"type": "ephemeral"},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "compaction", "content": "Earlier work was summarized."},
                    {"type": "text", "text": "Compacted answer"}
                ]},
                {"role": "user", "content": [{"type": "text", "text": "Continue"}]}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Final answer"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let completion = provider
        .complete_managed(ContextPlan {
            request: CompletionRequest {
                messages: vec![
                    Message::user("Old prompt that was compacted"),
                    Message::assistant("Old answer that was compacted"),
                    compacted.message,
                    Message::user("Continue"),
                ],
                tools: vec![],
            },
            size: ContextSize {
                current_tokens: 20_000,
                max_tokens: 60_000,
            },
            server_compaction_threshold: None,
            compacted: false,
        })
        .await
        .unwrap();

    assert_eq!(completion.message.content, "Final answer");
}

#[tokio::test]
async fn messages_api_rejects_a_failed_null_compaction_before_tool_execution() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [
                {"type": "compaction", "content": null},
                {"type":"tool_use","id":"summarizer-call","name":"write_file","input":{"path":"report.md"}}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let error = provider
        .complete_managed(ContextPlan {
            request: CompletionRequest {
                messages: vec![Message::user("Start")],
                tools: vec![ToolDefinition::new(
                    "write_file",
                    "Write a file",
                    json!({"type":"object"}),
                )],
            },
            size: ContextSize {
                current_tokens: 50_000,
                max_tokens: 60_000,
            },
            server_compaction_threshold: Some(40_000),
            compacted: false,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("compaction failed"));
}

#[tokio::test]
async fn messages_api_cache_optimizer_can_be_substituted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_json(json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "messages": [{"role":"user","content":[{"type":"text","text":"Hello"}]}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type":"message","role":"assistant","content":[{"type":"text","text":"Done"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key")
            .unwrap()
            .with_cache_optimizer(Arc::new(DisabledCacheOptimizer));

    provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: vec![],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn messages_api_omits_empty_assistant_retry_markers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_json(json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "cache_control": {"type": "ephemeral"},
            "messages": [
                {"role":"user","content":[{"type":"text","text":"First"}]},
                {"role":"user","content":[{"type":"text","text":"Retry"}]}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content":[{"type":"text","text":"Done"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let completion = provider
        .complete(CompletionRequest {
            messages: vec![
                Message::user("First"),
                Message::assistant(""),
                Message::user("Retry"),
            ],
            tools: vec![],
        })
        .await
        .unwrap();

    assert_eq!(completion.message, Message::assistant("Done"));
}

#[tokio::test]
async fn messages_api_does_not_follow_redirects_with_the_api_key() {
    let destination = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stolen"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": []})))
        .mount(&destination)
        .await;
    let origin = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/stolen", destination.uri())),
        )
        .mount(&origin)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(origin.uri(), "claude-test", "test-key").unwrap();

    provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: vec![],
        })
        .await
        .expect_err("redirects must fail instead of forwarding the API key");

    assert!(destination.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn messages_api_round_trips_ariadne_tools() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .and(body_json(json!({
            "model":"claude-test","max_tokens":4096,
            "cache_control":{"type":"ephemeral"},
            "messages":[
                {"role":"user","content":[{"type":"text","text":"Read it"}]},
                {"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"read_file","input":{"path":"README.md"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"{\"content\":\"# Ariadne\"}"}]}
            ],
            "tools":[{"name":"read_file","description":"Read a file","input_schema":{"type":"object"}}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "role":"assistant","content":[{"type":"tool_use","id":"call-2","name":"read_file","input":{"path":"Cargo.toml"}}]
        }))).expect(1).mount(&server).await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let result = provider
        .complete(CompletionRequest {
            messages: vec![
                Message::user("Read it"),
                Message::assistant_with_tool_calls(vec![ToolCall::new(
                    "call-1",
                    "read_file",
                    json!({"path":"README.md"}),
                )]),
                Message::tool("call-1", "{\"content\":\"# Ariadne\"}"),
            ],
            tools: vec![ToolDefinition::new(
                "read_file",
                "Read a file",
                json!({"type":"object"}),
            )],
        })
        .await
        .unwrap();
    assert_eq!(
        result.message.tool_calls,
        vec![ToolCall::new(
            "call-2",
            "read_file",
            json!({"path":"Cargo.toml"})
        )]
    );
}

#[tokio::test]
async fn messages_api_streams_text_and_accumulates_tool_input() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(concat!(
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        ))).expect(1).mount(&server).await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let mut deltas = vec![];
    let result = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |d| deltas.push(d.clone()),
        )
        .await
        .unwrap();
    assert_eq!(deltas, vec![CompletionDelta::Content("Hello".into())]);
    assert_eq!(result.message.content, "Hello");
    assert_eq!(
        result.message.tool_calls[0],
        ToolCall::new("call-1", "read_file", json!({"path":"README.md"}))
    );
}

#[tokio::test]
async fn messages_api_streaming_does_not_move_an_old_compaction_marker_forward() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-beta", "compact-2026-01-12"))
        .and(body_json(json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "cache_control": {"type": "ephemeral"},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "compaction", "content": "Earlier summary"},
                    {"type": "text", "text": "Earlier answer"}
                ]},
                {"role": "user", "content": [{"type": "text", "text": "Continue"}]}
            ],
            "stream": true
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Next answer\"}}\n\n",
                    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                )),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let mut compacted = Message::assistant("Earlier answer");
    compacted.provider_context = Some(ProviderContext::AnthropicCompaction(Some(
        "Earlier summary".to_owned(),
    )));

    let completion = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![compacted, Message::user("Continue")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap();

    assert_eq!(completion.message.content, "Next answer");
    assert_eq!(completion.message.provider_context, None);
}

#[tokio::test]
async fn messages_api_rejects_tool_delta_before_tool_start() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                )),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let error = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("before tool start"));
}

#[tokio::test]
async fn messages_api_rejects_tool_block_without_stop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                )),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let error = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("before tool block stop"));
}

#[tokio::test]
async fn messages_api_rejects_repeated_tool_block_stop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
                    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
                    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                )),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();

    let error = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("repeated a tool block stop"));
}

#[tokio::test]
async fn messages_api_rejects_tool_start_under_the_wrong_event_kind() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"message_delta\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(
        error.contains("tool start used the wrong event kind"),
        "{error}"
    );
}

#[tokio::test]
async fn messages_api_rejects_tool_start_carried_by_message_stop() {
    let error = malformed_stream_error(
        "data: {\"type\":\"message_stop\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
    )
    .await;

    assert!(
        error.contains("tool start used the wrong event kind"),
        "{error}"
    );
}

#[tokio::test]
async fn messages_api_rejects_tool_delta_carried_by_message_stop() {
    let error = malformed_stream_error(
        "data: {\"type\":\"message_stop\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
    )
    .await;

    assert!(
        error.contains("tool delta used the wrong event kind"),
        "{error}"
    );
}

#[tokio::test]
async fn messages_api_rejects_tool_delta_under_the_wrong_event_kind() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
        "data: {\"type\":\"message_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(
        error.contains("tool delta used the wrong event kind"),
        "{error}"
    );
}

#[tokio::test]
async fn messages_api_rejects_tool_start_without_an_index() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(error.contains("tool start is missing its index"), "{error}");
}

#[tokio::test]
async fn messages_api_rejects_tool_delta_without_an_index() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(error.contains("tool delta is missing its index"), "{error}");
}

#[tokio::test]
async fn messages_api_rejects_tool_delta_after_stop() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(error.contains("after tool block stop"), "{error}");
}

#[tokio::test]
async fn messages_api_rejects_content_block_stop_without_start() {
    let error = malformed_stream_error(concat!(
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))
    .await;

    assert!(
        error.contains("block stop arrived before block start"),
        "{error}"
    );
}

#[tokio::test]
async fn messages_api_propagates_stream_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "event: error\n",
                    "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded test-key\"}}\n\n"
                )),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let error = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Overloaded"));
    assert!(!error.to_string().contains("test-key"));
}

#[tokio::test]
async fn messages_api_ignores_unknown_stream_events_and_deltas() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "event: future_event\ndata: {\"type\":\"future_event\",\"future\":true}\n\n",
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"future_delta\",\"value\":1}}\n\n",
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
                )),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let completion = provider
        .complete_stream(
            CompletionRequest {
                messages: vec![Message::user("Hello")],
                tools: vec![],
            },
            &mut |_| {},
        )
        .await
        .unwrap();

    assert_eq!(completion.message, Message::assistant(""));
}

#[tokio::test]
async fn oversized_error_response_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_bytes(vec![b'x'; 1024 * 1024 + 1]))
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", "test-key").unwrap();
    let error = provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: vec![],
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("1048576-byte limit"));
}

#[tokio::test]
async fn error_responses_cannot_echo_the_api_key() {
    let server = MockServer::start().await;
    let secret = "test-secret-api-key";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string(format!("invalid credential: {secret}")),
        )
        .mount(&server)
        .await;
    let provider =
        AnthropicMessagesProvider::with_base_url(server.uri(), "claude-test", secret).unwrap();

    let error = provider
        .complete(CompletionRequest {
            messages: vec![Message::user("Hello")],
            tools: vec![],
        })
        .await
        .expect_err("the server rejected the credential");

    assert!(!error.to_string().contains(secret));
    assert!(error.to_string().contains("[REDACTED]"));
}
