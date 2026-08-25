use std::env;
use std::sync::Arc;

use ariadne_config::{ProfileCatalog, ProviderKind, ResolvedProfile};
use ariadne_core::{Agent, AgentProfiles, Message, ModelProvider, Profile};
use ariadne_provider_openai::OpenAiCompatibleProvider;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Deserialize)]
pub struct RespondRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub history: Vec<Message>,
}

#[derive(Debug, Serialize)]
pub struct RespondResponse {
    pub message: Message,
}

#[derive(Debug, Serialize)]
pub struct ProfilesResponse {
    pub default_profile: String,
    pub profiles: Vec<Profile>,
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

pub async fn respond_with_profiles(
    profiles: &AgentProfiles,
    request: RespondRequest,
) -> Result<RespondResponse, String> {
    let message = profiles
        .respond(
            request.profile.as_deref(),
            &request.history,
            &request.prompt,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(RespondResponse { message })
}

pub fn list_profiles(profiles: &AgentProfiles) -> ProfilesResponse {
    ProfilesResponse {
        default_profile: profiles.default_profile().to_owned(),
        profiles: profiles.profiles(),
    }
}

#[tauri::command]
async fn respond(
    profiles: State<'_, AgentProfiles>,
    request: RespondRequest,
) -> Result<RespondResponse, String> {
    respond_with_profiles(&profiles, request).await
}

#[tauri::command]
fn profiles(profiles: State<'_, AgentProfiles>) -> ProfilesResponse {
    list_profiles(&profiles)
}

pub fn run() {
    let configured = configured_profiles()
        .unwrap_or_else(|error| panic!("failed to configure Ariadne model provider: {error}"));

    tauri::Builder::default()
        .manage(configured)
        .invoke_handler(tauri::generate_handler![respond, profiles])
        .run(tauri::generate_context!())
        .expect("failed to run Ariadne desktop application");
}

fn configured_profiles() -> Result<AgentProfiles, String> {
    let catalog = match optional_env("ARIADNE_CONFIG")? {
        Some(path) => ProfileCatalog::load(path),
        None => ProfileCatalog::load_default(),
    }
    .map_err(|error| error.to_string())?;
    let default_profile =
        optional_env("ARIADNE_PROFILE")?.unwrap_or_else(|| catalog.default_profile().to_owned());
    catalog
        .resolve(&default_profile)
        .map_err(|error| error.to_string())?;

    let mut configured = Vec::new();
    for mut profile in catalog.resolve_all().map_err(|error| error.to_string())? {
        let api_key_override = if profile.profile.name == default_profile {
            if let Some(api_base) = optional_env("ARIADNE_API_BASE")? {
                profile.api_base = api_base;
            }
            if let Some(model) = optional_env("ARIADNE_MODEL")? {
                profile.profile.model = model;
            }
            if let Some(system_prompt) = optional_env("ARIADNE_SYSTEM_PROMPT")? {
                profile.system_prompt = system_prompt;
            }
            optional_env("ARIADNE_API_KEY")?
        } else {
            None
        };
        let agent = configured_agent(&profile, api_key_override)?;
        configured.push((profile.profile, agent));
    }

    AgentProfiles::new(default_profile, configured).map_err(|error| error.to_string())
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    decode_optional_env(name, env::var(name))
}

fn decode_optional_env(
    name: &str,
    value: Result<String, env::VarError>,
) -> Result<Option<String>, String> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!(
            "environment variable `{name}` is not valid Unicode"
        )),
    }
}

fn configured_agent(
    profile: &ResolvedProfile,
    api_key_override: Option<String>,
) -> Result<Agent, String> {
    let api_key = match api_key_override {
        Some(api_key) => Some(api_key),
        None => profile
            .api_key_env
            .as_deref()
            .map(|name| {
                env::var(name).map_err(|_| {
                    format!(
                        "profile `{}` requires provider API key environment variable `{name}`",
                        profile.profile.name
                    )
                })
            })
            .transpose()?,
    };
    let provider: Arc<dyn ModelProvider> = match profile.provider_kind {
        ProviderKind::OpenAiCompatible => Arc::new(
            OpenAiCompatibleProvider::new(&profile.api_base, &profile.profile.model, api_key)
                .map_err(|error| error.to_string())?,
        ),
    };

    Ok(Agent::new(provider, profile.system_prompt.clone()))
}

#[cfg(test)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;

    use super::decode_optional_env;

    #[test]
    fn configured_environment_values_must_be_valid_unicode() {
        let error = decode_optional_env(
            "ARIADNE_CONFIG",
            Err(VarError::NotUnicode(OsString::from("invalid"))),
        )
        .unwrap_err();

        assert_eq!(
            error,
            "environment variable `ARIADNE_CONFIG` is not valid Unicode"
        );
    }
}
