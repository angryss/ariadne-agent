use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread")]
async fn run_text_removes_terminal_control_characters() {
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
        .args(["run", "--prompt", "Do the work", "--output", "text"])
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model");

    command
        .assert()
        .success()
        .stdout(predicate::eq("safe[2J]0;owned31m\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_error_body_removes_terminal_control_characters() {
    let server = MockServer::start().await;
    let malicious = "safe\u{1b}[2J\u{1b}]0;owned\u{7}\r\u{8}\u{9b}31m";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string(malicious))
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .args(["run", "--prompt", "Do the work", "--output", "text"])
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model");

    command.assert().failure().stderr(
        predicate::str::contains("safe[2J]0;owned31m")
            .and(predicate::str::contains('\u{1b}').not())
            .and(predicate::str::contains('\u{7}').not())
            .and(predicate::str::contains('\r').not())
            .and(predicate::str::contains('\u{8}').not())
            .and(predicate::str::contains('\u{9b}').not()),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_json_escapes_control_characters_without_changing_content() {
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
    let output = command
        .args(["run", "--prompt", "Do the work", "--output", "json"])
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\\u001b"));
    assert!(stdout.contains("\\u0007"));
    assert!(stdout.contains("\\r"));
    assert!(stdout.contains("\\b"));
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains('\u{7}'));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["message"]["content"], malicious);
}

#[tokio::test(flavor = "multi_thread")]
async fn run_emits_one_json_response_for_unattended_use() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are Ariadne."},
                {"role": "user", "content": "Do the work"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Automated."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .args(["run", "--prompt", "Do the work", "--output", "json"])
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model")
        .env("ARIADNE_SYSTEM_PROMPT", "You are Ariadne.");

    command
        .assert()
        .success()
        .stdout(predicate::eq(
            "{\"message\":{\"role\":\"assistant\",\"content\":\"Automated.\"}}\n",
        ))
        .stderr(predicate::str::is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn run_reads_the_prompt_from_stdin_when_the_flag_is_omitted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are Ariadne."},
                {"role": "user", "content": "Prompt from stdin"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Read it."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .args(["run", "--output", "json"])
        .write_stdin("Prompt from stdin\n")
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model")
        .env("ARIADNE_SYSTEM_PROMPT", "You are Ariadne.");

    command.assert().success().stdout(predicate::eq(
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Read it.\"}}\n",
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_keeps_diagnostics_off_json_stdout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Machine readable."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command
        .args(["run", "--prompt", "Do the work", "--output", "json"])
        .env("ARIADNE_API_BASE", format!("{}/v1", server.uri()))
        .env("ARIADNE_MODEL", "test-model")
        .env("RUST_LOG", "trace");

    command.assert().success().stdout(predicate::eq(
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Machine readable.\"}}\n",
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_uses_the_selected_profiles_provider_model_and_system_prompt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_json(json!({
            "model": "work-model",
            "messages": [
                {"role": "system", "content": "Work profile policy"},
                {"role": "user", "content": "Use my profile"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Profile selected."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            r#"
version = 1
default_profile = "local"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[providers.work]
kind = "openai-compatible"
api_base = "{server}/v1"

[profiles.local]
provider = "local"
model = "local-model"

[profiles.work]
provider = "work"
model = "work-model"
system_prompt = "Work profile policy"
active_skills = ["rust"]
mcp_servers = ["filesystem"]

[mcp_servers.filesystem]
transport = "stdio"
command = "mcp-filesystem"
"#,
            server = server.uri()
        ),
    )
    .unwrap();

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command.args([
        "--config",
        config.to_str().unwrap(),
        "--profile",
        "work",
        "run",
        "--prompt",
        "Use my profile",
    ]);

    command
        .assert()
        .success()
        .stdout(predicate::eq("Profile selected.\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_does_not_require_credentials_for_an_inactive_profile() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Local profile."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            r#"
version = 1
default_profile = "local"

[providers.local]
kind = "openai-compatible"
api_base = "{server}/v1"

[providers.remote]
kind = "openai-compatible"
api_base = "https://example.com/v1"
api_key_env = "ARIADNE_TEST_MISSING_REMOTE_KEY"

[profiles.local]
provider = "local"
model = "local-model"

[profiles.remote]
provider = "remote"
model = "remote-model"
"#,
            server = server.uri()
        ),
    )
    .unwrap();

    let mut command = Command::cargo_bin("ariadne").unwrap();
    command.args([
        "--config",
        config.to_str().unwrap(),
        "run",
        "--prompt",
        "Stay local",
    ]);

    command
        .assert()
        .success()
        .stdout(predicate::eq("Local profile.\n"));
}
