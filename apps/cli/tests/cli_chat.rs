use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread")]
async fn chat_removes_terminal_control_characters() {
    let server = MockServer::start().await;
    let malicious = "safe\u{1b}[2J\u{1b}]0;owned\u{7}\r\u{8}\u{9b}31m";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": malicious}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .arg("chat")
        .write_stdin("Hello\n/quit\n")
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model");

    command.assert().success().stdout(predicate::eq(
        "Ariadne interactive mode. Type /quit to exit.\nyou> ariadne> safe[2J]0;owned31m\nyou> ",
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_keeps_history_until_the_user_quits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are Ariadne."},
                {"role": "user", "content": "Hello"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Ready."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are Ariadne."},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Ready."},
                {"role": "user", "content": "Next"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Done."}
            }]
        })))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .arg("chat")
        .write_stdin("Hello\nNext\n/quit\n")
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model")
        .env("ARIADNE_SYSTEM_PROMPT", "You are Ariadne.");

    command.assert().success().stdout(
        predicate::str::contains("Ariadne interactive mode")
            .and(predicate::str::contains("Ready."))
            .and(predicate::str::contains("Done.")),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_alias_quits_without_contacting_the_provider() {
    let server = MockServer::start().await;
    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .arg("chat")
        .write_stdin("/exit\n")
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model");

    command.assert().success().stdout(predicate::eq(
        "Ariadne interactive mode. Type /quit to exit.\nyou> ",
    ));
}
