use ariadne_core::{CompletionDelta, CompletionRequest, Message, ModelProvider};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ariadne_provider_openai::OpenAiCompatibleProvider;

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
async fn complete_calls_the_openai_compatible_chat_endpoint() {
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

    let provider = OpenAiCompatibleProvider::new(
        format!("{}/v1/", server.uri()),
        "test-model",
        Some("test-key".to_owned()),
    )
    .unwrap();

    let completion = provider
        .complete(CompletionRequest {
            messages: vec![Message::system("You are Ariadne."), Message::user("Hello")],
        })
        .await
        .unwrap();

    assert_eq!(
        completion.message,
        Message::assistant("Hello from the model")
    );
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
        })
        .await
        .expect_err("oversized error body must be rejected");

    assert!(
        error.to_string().contains("1048576-byte limit"),
        "unexpected error: {error}"
    );
}
