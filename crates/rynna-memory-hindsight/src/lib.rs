use async_trait::async_trait;
use reqwest::{Client, Url};
use rynna_config::memory::MemorySettings;
use rynna_core::{MemoryConversation, MemoryError, MemoryProvider, Role};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub struct HindsightMemoryProvider {
    client: Client,
    memories_url: Url,
    api_key: Option<String>,
    version_url: Url,
    append_supported: tokio::sync::OnceCell<bool>,
    legacy_document_prefix: uuid::Uuid,
}

/// Composition helper; the core depends only on MemoryProvider.
pub fn configured_memory(
    settings: &MemorySettings,
) -> Result<Option<Arc<dyn MemoryProvider>>, MemoryError> {
    match settings {
        MemorySettings::None => Ok(None),
        MemorySettings::Hindsight { .. } => {
            Ok(Some(Arc::new(HindsightMemoryProvider::new(settings)?)))
        }
    }
}

impl HindsightMemoryProvider {
    pub fn new(settings: &MemorySettings) -> Result<Self, MemoryError> {
        settings
            .validate()
            .map_err(|error| MemoryError(error.to_string()))?;
        let MemorySettings::Hindsight {
            api_base,
            bank_id,
            api_key,
            ..
        } = settings
        else {
            return Err(MemoryError("Hindsight settings are required".into()));
        };
        let mut memories_url =
            Url::parse(api_base).map_err(|_| MemoryError("invalid Hindsight URL".into()))?;
        let mut version_url = memories_url.clone();
        version_url
            .path_segments_mut()
            .expect("validated URL")
            .pop_if_empty()
            .push("version");
        memories_url
            .path_segments_mut()
            .map_err(|_| MemoryError("invalid Hindsight URL".into()))?
            .pop_if_empty()
            .extend(["v1", "default", "banks", bank_id, "memories"]);
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| MemoryError("could not create Hindsight client".into()))?;
        Ok(Self {
            client,
            memories_url,
            api_key: api_key.clone(),
            version_url,
            append_supported: tokio::sync::OnceCell::new(),
            legacy_document_prefix: uuid::Uuid::new_v4(),
        })
    }

    async fn supports_append(&self) -> bool {
        *self
            .append_supported
            .get_or_init(|| async {
                let mut request = self.client.get(self.version_url.clone());
                if let Some(key) = &self.api_key {
                    request = request.bearer_auth(key);
                }
                let Ok(mut response) = request.send().await else {
                    return false;
                };
                if !response.status().is_success() {
                    return false;
                }
                let mut bytes = Vec::new();
                while let Ok(Some(chunk)) = response.chunk().await {
                    if bytes.len() + chunk.len() > 4096 {
                        return false;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                    return false;
                };
                value
                    .get("version")
                    .or_else(|| value.get("api_version"))
                    .and_then(Value::as_str)
                    .and_then(|version| semver::Version::parse(version).ok())
                    .is_some_and(|version| version >= semver::Version::new(0, 5, 0))
            })
            .await
    }

    async fn post(&self, url: Url, body: Value) -> Result<reqwest::Response, MemoryError> {
        let mut request = self.client.post(url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|_| MemoryError("Hindsight request failed".into()))?;
        if !response.status().is_success() {
            // Remote errors may echo credentials or conversation text.
            return Err(MemoryError(format!(
                "Hindsight returned HTTP {}",
                response.status().as_u16()
            )));
        }
        Ok(response)
    }
}

#[derive(Deserialize)]
struct RecallResponse {
    results: Vec<RecalledMemory>,
}
#[derive(Deserialize)]
struct RecalledMemory {
    text: String,
}

#[async_trait]
impl MemoryProvider for HindsightMemoryProvider {
    async fn recall(&self, query: &str) -> Result<Vec<String>, MemoryError> {
        let mut url = self.memories_url.clone();
        url.path_segments_mut()
            .expect("validated base URL")
            .push("recall");
        let mut response = self
            .post(
                url,
                json!({"query": query, "max_tokens": 2048, "budget": "low"}),
            )
            .await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MemoryError("could not read Hindsight recall".into()))?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(MemoryError("Hindsight recall exceeds size limit".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let recalled: RecallResponse = serde_json::from_slice(&bytes)
            .map_err(|_| MemoryError("invalid Hindsight recall response".into()))?;
        Ok(recalled
            .results
            .into_iter()
            .take(32)
            .map(|fact| fact.text.chars().take(12_000).collect())
            .collect())
    }

    async fn retain(&self, conversation: &MemoryConversation) -> Result<(), MemoryError> {
        if conversation.messages.len() < 2 {
            return Err(MemoryError(
                "a completed memory exchange is required".into(),
            ));
        }
        // Hermes uses session-scoped append on Hindsight >= 0.5.0. Legacy APIs
        // replace a full transcript in a process-scoped document to avoid
        // overwriting a previous process's document on resume.
        let append = self.supports_append().await;
        let messages = if append {
            &conversation.messages[conversation.messages.len() - 2..]
        } else {
            &conversation.messages[..]
        };
        let session_id = conversation.session_id.to_string();
        let document_id = if append {
            format!("rynna-{session_id}")
        } else {
            format!("rynna-{session_id}-{}", self.legacy_document_prefix)
        };
        let mut item = json!({
            "content": serde_json::to_string(messages).expect("memory messages serialize"),
            "context": "conversation between Rynna and the User",
            "document_id": document_id,
            "timestamp": conversation.timestamp,
            "metadata": {
                "source": "rynna", "session_id": session_id,
                "retained_at": conversation.timestamp,
                "message_count": messages.len().to_string(),
                "turn_index": conversation.messages.iter().filter(|m| m.role == Role::Assistant).count().to_string()
            },
            "tags": [format!("session:{session_id}")]
        });
        if append {
            item["update_mode"] = json!("append");
        }
        self.post(
            self.memories_url.clone(),
            json!({
                "items": [item], "async": true
            }),
        )
        .await?;
        Ok(())
    }
}
