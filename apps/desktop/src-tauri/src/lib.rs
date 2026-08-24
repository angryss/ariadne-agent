use std::env;
use std::sync::Arc;

use ariadne_core::{Agent, Message, ModelProvider};
use ariadne_provider_openai::OpenAiCompatibleProvider;
use serde::{Deserialize, Serialize};
use tauri::State;

const DEFAULT_API_BASE: &str = "http://127.0.0.1:11434/v1";
const DEFAULT_MODEL: &str = "qwen3:8b";
const DEFAULT_SYSTEM_PROMPT: &str = "You are Ariadne, a careful and capable AI software agent.";

#[derive(Deserialize)]
pub struct RespondRequest {
    pub prompt: String,
    #[serde(default)]
    pub history: Vec<Message>,
}

#[derive(Debug, Serialize)]
pub struct RespondResponse {
    pub message: Message,
}

pub async fn respond_with_agent(
    agent: &Agent,
    request: RespondRequest,
) -> Result<RespondResponse, String> {
    let message = agent
        .respond(&request.history, &request.prompt)
        .await
        .map_err(|error| error.to_string())?;
    Ok(RespondResponse { message })
}

#[tauri::command]
async fn respond(
    agent: State<'_, Agent>,
    request: RespondRequest,
) -> Result<RespondResponse, String> {
    respond_with_agent(&agent, request).await
}

pub fn run() {
    let agent = configured_agent()
        .unwrap_or_else(|error| panic!("failed to configure Ariadne model provider: {error}"));

    tauri::Builder::default()
        .manage(agent)
        .invoke_handler(tauri::generate_handler![respond])
        .run(tauri::generate_context!())
        .expect("failed to run Ariadne desktop application");
}

fn configured_agent() -> Result<Agent, String> {
    let api_base = env::var("ARIADNE_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_owned());
    let model = env::var("ARIADNE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let api_key = env::var("ARIADNE_API_KEY").ok();
    let system_prompt =
        env::var("ARIADNE_SYSTEM_PROMPT").unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_owned());
    let provider = OpenAiCompatibleProvider::new(api_base, model, api_key)
        .map_err(|error| error.to_string())?;

    Ok(Agent::new(
        Arc::new(provider) as Arc<dyn ModelProvider>,
        system_prompt,
    ))
}
