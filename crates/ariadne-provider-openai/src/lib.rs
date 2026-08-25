use std::time::Duration;

use ariadne_core::{Completion, CompletionRequest, ModelProvider, ProviderError, Role};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_BODY_CHARS: usize = 512;
// Non-streaming chat completions should fit comfortably in 1 MiB while bounding memory use.
const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

pub struct OpenAiCompatibleProvider {
    client: Client,
    completion_url: Url,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: impl AsRef<str>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Result<Self, ProviderConfigError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(ProviderConfigError::BlankModel);
        }

        let endpoint = format!(
            "{}/chat/completions",
            base_url.as_ref().trim_end_matches('/')
        );
        let completion_url = Url::parse(&endpoint)
            .map_err(|error| ProviderConfigError::InvalidBaseUrl(error.to_string()))?;
        let api_key = api_key.filter(|key| !key.trim().is_empty());
        if !completion_url.username().is_empty() || completion_url.password().is_some() {
            return Err(ProviderConfigError::EmbeddedCredentials);
        }
        if !matches!(completion_url.scheme(), "http" | "https") {
            return Err(ProviderConfigError::UnsupportedScheme);
        }
        if api_key.is_some()
            && completion_url.scheme() == "http"
            && !matches!(
                completion_url.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]")
            )
        {
            return Err(ProviderConfigError::InsecureCredentials);
        }
        let client = build_http_client(DEFAULT_TIMEOUT)?;

        Ok(Self {
            client,
            completion_url,
            model,
            api_key,
        })
    }
}

fn build_http_client(io_timeout: Duration) -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(io_timeout)
        .read_timeout(io_timeout)
        .build()
}

#[derive(Debug, Error)]
pub enum ProviderConfigError {
    #[error("provider base URL is invalid: {0}")]
    InvalidBaseUrl(String),
    #[error("provider base URL must not contain embedded credentials")]
    EmbeddedCredentials,
    #[error("provider base URL must use HTTP or HTTPS")]
    UnsupportedScheme,
    #[error("provider credentials require HTTPS except for loopback HTTP endpoints")]
    InsecureCredentials,
    #[error("provider model must not be blank")]
    BlankModel,
    #[error("failed to build HTTP client: {0}")]
    Client(#[from] reqwest::Error),
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ariadne_core::Message],
    #[serde(skip_serializing_if = "is_false")]
    stream: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ariadne_core::Message,
}

#[derive(Deserialize)]
struct ChatCompletionChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: &request.messages,
            stream: false,
        };
        let mut request_builder = self.client.post(self.completion_url.clone()).json(&payload);
        if let Some(api_key) = &self.api_key {
            request_builder = request_builder.bearer_auth(api_key);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| ProviderError::new(format!("request failed: {error}")))?;
        let status = response.status();

        if !status.is_success() {
            let body = read_response_body(response).await?;
            let body = String::from_utf8_lossy(&body);
            return Err(ProviderError::new(format!(
                "provider returned {status}: {}",
                truncate(&body, MAX_ERROR_BODY_CHARS)
            )));
        }

        let body = read_response_body(response).await?;
        let response: ChatCompletionResponse = serde_json::from_slice(&body)
            .map_err(|error| ProviderError::new(format!("invalid provider response: {error}")))?;
        let message = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::new("provider returned no choices"))?
            .message;

        if message.role != Role::Assistant {
            return Err(ProviderError::new(
                "provider response message must have the assistant role",
            ));
        }

        Ok(Completion::new(message))
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta str) + Send),
    ) -> Result<Completion, ProviderError> {
        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: &request.messages,
            stream: true,
        };
        let mut request_builder = self.client.post(self.completion_url.clone()).json(&payload);
        if let Some(api_key) = &self.api_key {
            request_builder = request_builder.bearer_auth(api_key);
        }

        let mut response = request_builder
            .send()
            .await
            .map_err(|error| ProviderError::new(format!("request failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = read_response_body(response).await?;
            let body = String::from_utf8_lossy(&body);
            return Err(ProviderError::new(format!(
                "provider returned {status}: {}",
                truncate(&body, MAX_ERROR_BODY_CHARS)
            )));
        }

        let mut pending = Vec::new();
        let mut content = String::new();
        let mut received = 0usize;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| ProviderError::new(format!("failed to read response: {error}")))?
        {
            received = received.saturating_add(chunk.len());
            if received > MAX_RESPONSE_BODY_BYTES {
                return Err(response_body_too_large());
            }
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let mut line = pending.drain(..=newline).collect::<Vec<_>>();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                process_sse_line(&line, &mut content, on_delta)?;
            }
        }
        if !pending.is_empty() {
            process_sse_line(&pending, &mut content, on_delta)?;
        }

        Ok(Completion::new(ariadne_core::Message::assistant(content)))
    }
}

fn process_sse_line(
    line: &[u8],
    content: &mut String,
    on_delta: &mut (dyn for<'delta> FnMut(&'delta str) + Send),
) -> Result<(), ProviderError> {
    let Some(data) = line.strip_prefix(b"data:") else {
        return Ok(());
    };
    let data = data.strip_prefix(b" ").unwrap_or(data);
    if data == b"[DONE]" || data.is_empty() {
        return Ok(());
    }
    let chunk: ChatCompletionChunk = serde_json::from_slice(data)
        .map_err(|error| ProviderError::new(format!("invalid provider stream: {error}")))?;
    for choice in chunk.choices {
        if let Some(delta) = choice.delta.content {
            on_delta(&delta);
            content.push_str(&delta);
        }
    }
    Ok(())
}

async fn read_response_body(mut response: reqwest::Response) -> Result<Vec<u8>, ProviderError> {
    let content_length = response.content_length().unwrap_or(0);
    if content_length > MAX_RESPONSE_BODY_BYTES as u64 {
        return Err(response_body_too_large());
    }

    let mut body = Vec::with_capacity(content_length as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ProviderError::new(format!("failed to read response: {error}")))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
            return Err(response_body_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn response_body_too_large() -> ProviderError {
    ProviderError::new(format!(
        "provider response exceeded {MAX_RESPONSE_BODY_BYTES}-byte limit"
    ))
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use ariadne_core::{CompletionRequest, Message, ModelProvider};
    use reqwest::Url;

    use super::{OpenAiCompatibleProvider, build_http_client};

    #[tokio::test]
    async fn active_stream_can_outlive_the_per_read_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let request_bytes = connection.read(&mut request).unwrap();
            assert!(request_bytes > 0);
            connection
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();

            for body in [
                "data: {\"choices\":[{\"delta\":{\"content\":\"still\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\" stream\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"ing\"}}]}\n\n",
                "data: [DONE]\n\n",
            ] {
                write!(connection, "{:x}\r\n{body}\r\n", body.len()).unwrap();
                connection.flush().unwrap();
                thread::sleep(Duration::from_millis(40));
            }
            connection.write_all(b"0\r\n\r\n").unwrap();
        });
        let provider = OpenAiCompatibleProvider {
            client: build_http_client(Duration::from_millis(70)).unwrap(),
            completion_url: Url::parse(&format!("http://{address}/v1/chat/completions")).unwrap(),
            model: "test-model".to_owned(),
            api_key: None,
        };
        let mut deltas = Vec::new();

        let completion = provider
            .complete_stream(
                CompletionRequest {
                    messages: vec![Message::user("Hello")],
                },
                &mut |delta| deltas.push(delta.to_owned()),
            )
            .await
            .unwrap();

        server.join().unwrap();
        assert_eq!(deltas, ["still", " stream", "ing"]);
        assert_eq!(completion.message, Message::assistant("still streaming"));
    }
}
