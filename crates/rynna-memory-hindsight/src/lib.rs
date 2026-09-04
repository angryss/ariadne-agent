use async_trait::async_trait;
use reqwest::{Client, Url};
use rynna_config::memory::MemorySettings;
use rynna_core::{MemoryError, MemoryProvider};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub struct HindsightMemoryProvider {
    client: Client,
    memories_url: Url,
    api_key: Option<String>,
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
        })
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

    async fn retain(&self, input: &str, answer: &str) -> Result<(), MemoryError> {
        let content = format!("User: {input}\nAssistant: {answer}");
        self.post(
            self.memories_url.clone(),
            json!({
                "items": [{"content": content, "context": "Rynna conversation"}], "async": true
            }),
        )
        .await?;
        Ok(())
    }
}
