use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ariadne_config::{
    ConfiguredProvider, OpenAiAuthentication, ProfileCatalog, ProviderKind, ProviderSettingsStore,
    ResolvedCapability, ResolvedProfile, secure_private_directory,
};
use ariadne_core::{Agent, AgentProfiles, CompletionDelta, Message, ModelProvider, Profile, Tool};
use ariadne_provider_openai::OpenAiCompatibleProvider;
use ariadne_tools_command::{CommandConfig, CommandTool};
use ariadne_tools_filesystem::{FileSystemConfig, FileSystemToolset};
use serde::{Deserialize, Serialize};
use tauri::{State, ipc::Channel};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, timeout};

mod codex_provider;
pub use codex_provider::CodexAppServerProvider;

const MAX_CODEX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_API_KEY_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum OpenAiConnectRequest {
    Chatgpt,
    ApiKey { api_key: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct OpenAiAccountResponse {
    pub connected: bool,
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProviderInput {
    Ollama {
        api_base: String,
    },
    #[serde(rename = "openai")]
    OpenAi {
        authentication: OpenAiAuthentication,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        reuse_existing: bool,
    },
}

impl ProviderInput {
    fn kind(&self) -> &'static str {
        match self {
            Self::Ollama { .. } => "ollama",
            Self::OpenAi { .. } => "openai",
        }
    }
}

#[derive(Default)]
struct OpenAiAuthenticationLock(Mutex<()>);

impl OpenAiAuthenticationLock {
    async fn acquire(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }
}

#[derive(Clone)]
pub(crate) struct OpenAiCredentialSelection(Arc<AtomicBool>);

impl OpenAiCredentialSelection {
    fn new(reuse_existing: bool) -> Self {
        Self(Arc::new(AtomicBool::new(reuse_existing)))
    }

    pub(crate) fn reuses_existing(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set_reuse_existing(&self, reuse_existing: bool) {
        self.0.store(reuse_existing, Ordering::Release);
    }
}

pub async fn connect_openai_with_program(
    program: impl AsRef<OsStr>,
    request: OpenAiConnectRequest,
) -> Result<OpenAiAccountResponse, String> {
    connect_openai_with_program_and_optional_home(program.as_ref(), None, request).await
}

pub async fn connect_openai_with_program_and_home(
    program: impl AsRef<OsStr>,
    codex_home: impl AsRef<OsStr>,
    request: OpenAiConnectRequest,
) -> Result<OpenAiAccountResponse, String> {
    connect_openai_with_program_and_optional_home(
        program.as_ref(),
        Some(codex_home.as_ref()),
        request,
    )
    .await
}

async fn connect_openai_with_program_and_optional_home(
    program: &OsStr,
    codex_home: Option<&OsStr>,
    request: OpenAiConnectRequest,
) -> Result<OpenAiAccountResponse, String> {
    let mut command = Command::new(program);
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    command
        .arg("login")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match request {
        OpenAiConnectRequest::Chatgpt => {
            command.stdin(Stdio::null());
        }
        OpenAiConnectRequest::ApiKey { api_key } => {
            if api_key.trim().is_empty() {
                return Err("OpenAI API key must not be blank".to_owned());
            }
            if api_key.len() > MAX_API_KEY_BYTES {
                return Err("OpenAI API key is too large".to_owned());
            }
            command.arg("--with-api-key").stdin(Stdio::piped());
            let mut child = command
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| format!("failed to start Codex login: {error}"))?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or("Codex login stdin is unavailable")?;
            stdin
                .write_all(api_key.as_bytes())
                .await
                .map_err(|_| "failed to send the API key to Codex".to_owned())?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|_| "failed to send the API key to Codex".to_owned())?;
            drop(stdin);
            let status = timeout(Duration::from_secs(120), child.wait())
                .await
                .map_err(|_| "Codex API-key login timed out".to_owned())?
                .map_err(|error| format!("failed to wait for Codex login: {error}"))?;
            if !status.success() {
                return Err("Codex rejected the OpenAI API key".to_owned());
            }
            return match codex_home {
                Some(codex_home) => openai_account_with_program_and_home(program, codex_home).await,
                None => openai_account_with_program(program).await,
            };
        }
    }

    let mut child = command
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("failed to start ChatGPT login: {error}"))?;
    let status = timeout(Duration::from_secs(300), child.wait())
        .await
        .map_err(|_| "ChatGPT login timed out".to_owned())?
        .map_err(|error| format!("failed to wait for ChatGPT login: {error}"))?;
    if !status.success() {
        return Err("ChatGPT login did not complete".to_owned());
    }
    match codex_home {
        Some(codex_home) => openai_account_with_program_and_home(program, codex_home).await,
        None => openai_account_with_program(program).await,
    }
}

pub async fn openai_account_with_program(
    program: impl AsRef<OsStr>,
) -> Result<OpenAiAccountResponse, String> {
    let mut command = Command::new(program);
    command
        .env_remove("CODEX_HOME")
        .env_remove("ARIADNE_CODEX_HOME");
    openai_account_with_command(command).await
}

pub async fn openai_account_with_program_and_home(
    program: impl AsRef<OsStr>,
    codex_home: impl AsRef<OsStr>,
) -> Result<OpenAiAccountResponse, String> {
    let mut command = Command::new(program);
    command.env("CODEX_HOME", codex_home);
    openai_account_with_command(command).await
}

async fn openai_account_with_command(
    mut command: Command,
) -> Result<OpenAiAccountResponse, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut child = command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("failed to start Codex app-server: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Codex app-server stdin is unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Codex app-server stdout is unavailable")?;
    let mut stdout = BufReader::new(stdout);

    write_codex_message(
        &mut stdin,
        &serde_json::json!({
            "method": "initialize",
            "id": 1,
            "params": {"clientInfo": {"name": "ariadne", "title": "Ariadne", "version": env!("CARGO_PKG_VERSION")}}
        }),
        deadline,
    )
    .await?;
    read_codex_response(&mut stdout, 1, deadline).await?;
    write_codex_message(
        &mut stdin,
        &serde_json::json!({"method": "initialized", "params": {}}),
        deadline,
    )
    .await?;
    write_codex_message(
        &mut stdin,
        &serde_json::json!({"method": "account/read", "id": 2, "params": {"refreshToken": false}}),
        deadline,
    )
    .await?;
    let response = read_codex_response(&mut stdout, 2, deadline).await?;
    let account = response.pointer("/result/account");
    let Some(kind) = account
        .and_then(|account| account.get("type"))
        .and_then(|kind| kind.as_str())
    else {
        return Ok(OpenAiAccountResponse {
            connected: false,
            method: None,
            plan: None,
        });
    };
    let (method, plan) = match kind {
        "apiKey" => ("api_key", None),
        "chatgpt" => (
            "chatgpt",
            account
                .and_then(|account| account.get("planType"))
                .and_then(|plan| plan.as_str())
                .map(str::to_owned),
        ),
        _ => {
            return Ok(OpenAiAccountResponse {
                connected: false,
                method: None,
                plan: None,
            });
        }
    };
    Ok(OpenAiAccountResponse {
        connected: true,
        method: Some(method.to_owned()),
        plan,
    })
}

async fn write_codex_message(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &serde_json::Value,
    deadline: Instant,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    if encoded.len() > MAX_CODEX_MESSAGE_BYTES {
        return Err("Codex app-server request exceeded the size limit".to_owned());
    }
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| "Codex app-server request timed out".to_owned())?;
    timeout(remaining, writer.write_all(&encoded))
        .await
        .map_err(|_| "Codex app-server request timed out".to_owned())?
        .map_err(|error| format!("failed to write to Codex app-server: {error}"))
}

async fn read_codex_response(
    reader: &mut (impl AsyncBufRead + Unpin),
    id: u64,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    loop {
        let message = read_codex_message(reader, deadline).await?;
        if message.get("id").and_then(|value| value.as_u64()) == Some(id) {
            if let Some(error) = message
                .pointer("/error/message")
                .and_then(|value| value.as_str())
            {
                return Err(format!("Codex app-server request failed: {error}"));
            }
            return Ok(message);
        }
    }
}

pub(crate) async fn read_codex_message(
    reader: &mut (impl AsyncBufRead + Unpin),
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    let mut line = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Codex app-server response timed out".to_owned())?;
        let available = timeout(remaining, reader.fill_buf())
            .await
            .map_err(|_| "Codex app-server response timed out".to_owned())?
            .map_err(|error| format!("failed to read Codex app-server response: {error}"))?;
        if available.is_empty() {
            if line.is_empty() {
                return Err("Codex app-server stopped unexpectedly".to_owned());
            }
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if line.len().saturating_add(take) > MAX_CODEX_MESSAGE_BYTES {
            return Err("Codex app-server message exceeded the size limit".to_owned());
        }
        let complete = available[take - 1] == b'\n';
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if complete {
            break;
        }
    }
    serde_json::from_slice(&line).map_err(|_| "Codex app-server returned invalid JSON".to_owned())
}

fn codex_program() -> PathBuf {
    env::var_os("ARIADNE_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"))
}

pub fn prepare_codex_home(
    config_directory: impl AsRef<std::path::Path>,
) -> Result<PathBuf, String> {
    secure_codex_home(config_directory.as_ref().join("ariadne").join("codex"))
}

fn configured_codex_home() -> Result<PathBuf, String> {
    match env::var_os("ARIADNE_CODEX_HOME") {
        Some(home) => secure_codex_home(PathBuf::from(home)),
        None => prepare_codex_home(
            dirs::config_dir().ok_or("Ariadne could not determine its configuration directory")?,
        ),
    }
}

fn selected_openai_codex_home(
    credential_selection: &OpenAiCredentialSelection,
    private_home: &std::path::Path,
) -> Option<PathBuf> {
    (!credential_selection.reuses_existing()).then(|| private_home.to_path_buf())
}

pub(crate) fn secure_codex_home(home: PathBuf) -> Result<PathBuf, String> {
    secure_private_directory(home).map_err(|error| {
        if error.to_string().contains("symbolic link") {
            "Ariadne's Codex directory must not be a symbolic link or contain symbolic links"
                .to_owned()
        } else {
            format!("failed to prepare Ariadne's Codex directory: {error}")
        }
    })
}

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

pub async fn respond_stream_with_profiles(
    profiles: &AgentProfiles,
    request: RespondRequest,
    on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
) -> Result<RespondResponse, String> {
    let message = profiles
        .respond_stream(
            request.profile.as_deref(),
            &request.history,
            &request.prompt,
            on_delta,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(RespondResponse { message })
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionDeltaEvent {
    Thinking { content: String },
    Content { content: String },
}

impl From<&CompletionDelta> for CompletionDeltaEvent {
    fn from(delta: &CompletionDelta) -> Self {
        match delta {
            CompletionDelta::Thinking(content) => Self::Thinking {
                content: content.clone(),
            },
            CompletionDelta::Content(content) => Self::Content {
                content: content.clone(),
            },
        }
    }
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
async fn respond_stream(
    profiles: State<'_, AgentProfiles>,
    request: RespondRequest,
    on_event: Channel<CompletionDeltaEvent>,
) -> Result<RespondResponse, String> {
    let mut on_delta = |delta: &CompletionDelta| {
        let _ = on_event.send(CompletionDeltaEvent::from(delta));
    };
    respond_stream_with_profiles(&profiles, request, &mut on_delta).await
}

#[tauri::command]
fn profiles(profiles: State<'_, AgentProfiles>) -> ProfilesResponse {
    list_profiles(&profiles)
}

#[tauri::command]
async fn openai_account(
    credential_selection: State<'_, OpenAiCredentialSelection>,
) -> Result<OpenAiAccountResponse, String> {
    let private_home = configured_codex_home()?;
    match selected_openai_codex_home(&credential_selection, &private_home) {
        Some(home) => openai_account_with_program_and_home(codex_program(), home).await,
        None => openai_account_with_program(codex_program()).await,
    }
}

#[tauri::command]
async fn existing_openai_account() -> Result<OpenAiAccountResponse, String> {
    openai_account_with_program(codex_program()).await
}

#[tauri::command]
async fn connect_openai(
    authentication_lock: State<'_, OpenAiAuthenticationLock>,
    credential_selection: State<'_, OpenAiCredentialSelection>,
    provider_settings: State<'_, Mutex<ProviderSettingsStore>>,
    request: OpenAiConnectRequest,
) -> Result<OpenAiAccountResponse, String> {
    let _authentication = authentication_lock.acquire().await;
    let provider_authentication = match &request {
        OpenAiConnectRequest::Chatgpt => OpenAiAuthentication::Chatgpt,
        OpenAiConnectRequest::ApiKey { .. } => OpenAiAuthentication::ApiKey,
    };
    let account =
        connect_openai_with_program_and_home(codex_program(), configured_codex_home()?, request)
            .await?;
    let mut store = provider_settings.lock().await;
    store.refresh().map_err(|error| error.to_string())?;
    synchronize_connected_openai_provider(
        &mut store,
        &credential_selection,
        provider_authentication,
    )?;
    Ok(account)
}

#[tauri::command]
async fn list_providers(
    provider_settings: State<'_, Mutex<ProviderSettingsStore>>,
) -> Result<Vec<ConfiguredProvider>, String> {
    let mut store = provider_settings.lock().await;
    store.refresh().map_err(|error| error.to_string())?;
    Ok(store.list())
}

async fn configured_provider_from_input(
    input: ProviderInput,
) -> Result<ConfiguredProvider, String> {
    match input {
        ProviderInput::Ollama { api_base } => Ok(ConfiguredProvider::Ollama { api_base }),
        ProviderInput::OpenAi {
            authentication,
            api_key,
            reuse_existing,
        } => {
            if reuse_existing {
                if authentication != OpenAiAuthentication::Chatgpt {
                    return Err(
                        "only ChatGPT subscriptions can reuse existing credentials".to_owned()
                    );
                }
                verify_existing_openai_credentials_with_program(codex_program()).await?;
                return Ok(ConfiguredProvider::OpenAi {
                    authentication,
                    reuse_existing: true,
                });
            }
            let request = match authentication {
                OpenAiAuthentication::Chatgpt => OpenAiConnectRequest::Chatgpt,
                OpenAiAuthentication::ApiKey => OpenAiConnectRequest::ApiKey {
                    api_key: api_key.ok_or("OpenAI API key is required")?,
                },
            };
            connect_openai_with_program_and_home(
                codex_program(),
                configured_codex_home()?,
                request,
            )
            .await?;
            Ok(ConfiguredProvider::OpenAi {
                authentication,
                reuse_existing: false,
            })
        }
    }
}

async fn verify_existing_openai_credentials_with_program(
    program: impl AsRef<OsStr>,
) -> Result<(), String> {
    let output = timeout(
        Duration::from_secs(30),
        Command::new(program)
            .args(["login", "status"])
            .env_remove("CODEX_HOME")
            .env_remove("ARIADNE_CODEX_HOME")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "OpenAI credential check timed out".to_owned())?
    .map_err(|error| format!("failed to check existing ChatGPT credentials: {error}"))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).contains("Logged in using ChatGPT")
    {
        return Err("existing ChatGPT credentials are unavailable".to_owned());
    }
    Ok(())
}

#[tauri::command]
async fn create_provider(
    authentication_lock: State<'_, OpenAiAuthenticationLock>,
    credential_selection: State<'_, OpenAiCredentialSelection>,
    provider_settings: State<'_, Mutex<ProviderSettingsStore>>,
    provider: ProviderInput,
) -> Result<ConfiguredProvider, String> {
    let _authentication = authentication_lock.acquire().await;
    let mut store = provider_settings.lock().await;
    store.refresh().map_err(|error| error.to_string())?;
    if store.get(provider.kind()).is_some() {
        return Err(format!(
            "provider `{}` is already configured",
            provider.kind()
        ));
    }
    let provider = configured_provider_from_input(provider).await?;
    store
        .add(provider.clone())
        .map_err(|error| error.to_string())?;
    update_openai_credential_selection(&credential_selection, &provider);
    Ok(provider)
}

#[tauri::command]
async fn update_provider(
    authentication_lock: State<'_, OpenAiAuthenticationLock>,
    credential_selection: State<'_, OpenAiCredentialSelection>,
    provider_settings: State<'_, Mutex<ProviderSettingsStore>>,
    provider: ProviderInput,
) -> Result<ConfiguredProvider, String> {
    let _authentication = authentication_lock.acquire().await;
    let mut store = provider_settings.lock().await;
    store.refresh().map_err(|error| error.to_string())?;
    if store.get(provider.kind()).is_none() {
        return Err(format!("provider `{}` is not configured", provider.kind()));
    }
    let provider = configured_provider_from_input(provider).await?;
    store
        .update(provider.clone())
        .map_err(|error| error.to_string())?;
    update_openai_credential_selection(&credential_selection, &provider);
    Ok(provider)
}

#[tauri::command]
async fn delete_provider(
    credential_selection: State<'_, OpenAiCredentialSelection>,
    provider_settings: State<'_, Mutex<ProviderSettingsStore>>,
    kind: String,
) -> Result<(), String> {
    provider_settings
        .lock()
        .await
        .delete(&kind)
        .map_err(|error| error.to_string())?;
    if kind == "openai" {
        credential_selection.set_reuse_existing(false);
    }
    Ok(())
}

pub fn run() {
    let provider_settings = configured_provider_settings()
        .unwrap_or_else(|error| panic!("failed to load Ariadne provider settings: {error}"));
    let credential_selection = OpenAiCredentialSelection::new(
        openai_account_reuses_existing_credentials(&provider_settings),
    );
    let configured = configured_profiles(credential_selection.clone())
        .unwrap_or_else(|error| panic!("failed to configure Ariadne model provider: {error}"));

    tauri::Builder::default()
        .manage(configured)
        .manage(credential_selection)
        .manage(Mutex::new(provider_settings))
        .manage(OpenAiAuthenticationLock::default())
        .invoke_handler(tauri::generate_handler![
            respond,
            respond_stream,
            profiles,
            openai_account,
            existing_openai_account,
            connect_openai,
            list_providers,
            create_provider,
            update_provider,
            delete_provider
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Ariadne desktop application");
}

fn configured_provider_settings() -> Result<ProviderSettingsStore, String> {
    match env::var_os("ARIADNE_PROVIDER_CONFIG") {
        Some(path) => ProviderSettingsStore::load(PathBuf::from(path)),
        None => ProviderSettingsStore::load_default(),
    }
    .map_err(|error| error.to_string())
}

fn configured_profiles(
    credential_selection: OpenAiCredentialSelection,
) -> Result<AgentProfiles, String> {
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

    const OPENAI_ACCOUNT_PROFILE: &str = "openai-account";
    if configured
        .iter()
        .any(|(profile, _)| profile.name == OPENAI_ACCOUNT_PROFILE)
    {
        return Err(format!(
            "profile name `{OPENAI_ACCOUNT_PROFILE}` is reserved for the desktop OpenAI account"
        ));
    }
    let openai_profile = Profile {
        name: OPENAI_ACCOUNT_PROFILE.to_owned(),
        provider: "openai".to_owned(),
        model: "Codex default".to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
        capabilities: Vec::new(),
    };
    let openai_provider: Arc<dyn ModelProvider> =
        Arc::new(CodexAppServerProvider::with_selectable_home(
            codex_program(),
            configured_codex_home()?,
            credential_selection,
            None,
        ));
    let openai_agent = Agent::new(
        openai_provider,
        "You are Ariadne, a careful and capable AI software agent.",
    );
    configured.push((openai_profile, openai_agent));

    AgentProfiles::new(default_profile, configured).map_err(|error| error.to_string())
}

fn openai_account_reuses_existing_credentials(provider_settings: &ProviderSettingsStore) -> bool {
    provider_settings
        .get("openai")
        .is_some_and(configured_provider_reuses_existing_credentials)
}

fn configured_provider_reuses_existing_credentials(provider: &ConfiguredProvider) -> bool {
    matches!(
        provider,
        ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::Chatgpt,
            reuse_existing: true,
        }
    )
}

fn update_openai_credential_selection(
    credential_selection: &OpenAiCredentialSelection,
    provider: &ConfiguredProvider,
) {
    if matches!(provider, ConfiguredProvider::OpenAi { .. }) {
        credential_selection
            .set_reuse_existing(configured_provider_reuses_existing_credentials(provider));
    }
}

fn synchronize_connected_openai_provider(
    provider_settings: &mut ProviderSettingsStore,
    credential_selection: &OpenAiCredentialSelection,
    authentication: OpenAiAuthentication,
) -> Result<(), String> {
    if provider_settings.get("openai").is_some() {
        provider_settings
            .update(ConfiguredProvider::OpenAi {
                authentication,
                reuse_existing: false,
            })
            .map_err(|error| error.to_string())?;
    }
    credential_selection.set_reuse_existing(false);
    Ok(())
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

    compose_agent(profile, provider)
}

#[doc(hidden)]
pub fn compose_agent(
    profile: &ResolvedProfile,
    provider: Arc<dyn ModelProvider>,
) -> Result<Agent, String> {
    let tools = configured_tools(profile)?;
    if tools.is_empty() {
        Ok(Agent::new(provider, profile.system_prompt.clone()))
    } else {
        Agent::with_tools(provider, profile.system_prompt.clone(), tools)
            .map_err(|error| error.to_string())
    }
}

fn configured_tools(profile: &ResolvedProfile) -> Result<Vec<Arc<dyn Tool>>, String> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for capability in &profile.capabilities {
        match capability {
            ResolvedCapability::Command(capability) => {
                tools.push(Arc::new(
                    CommandTool::new(CommandConfig {
                        working_directory: capability.working_directory.clone(),
                        programs: capability.programs.clone(),
                        timeout_seconds: capability.timeout_seconds,
                        max_output_bytes: capability.max_output_bytes,
                    })
                    .map_err(|error| error.to_string())?,
                ));
            }
            ResolvedCapability::FileSystem(capability) => {
                let mut config = FileSystemConfig::new(&capability.root);
                config.read_only = capability.read_only;
                config.allowed_patterns = capability.allowed_patterns.clone();
                if let Some(patterns) = &capability.denied_patterns {
                    config.denied_patterns.clone_from(patterns);
                }
                if let Some(patterns) = &capability.protected_patterns {
                    config.protected_patterns.clone_from(patterns);
                }
                if let Some(limit) = capability.max_read_bytes {
                    config.max_read_bytes = limit;
                }
                if let Some(limit) = capability.max_results {
                    config.max_results = limit;
                }
                if let Some(limit) = capability.max_traversal_files {
                    config.max_traversal_files = limit;
                }
                if let Some(limit) = capability.max_traversal_depth {
                    config.max_traversal_depth = limit;
                }
                if let Some(limit) = capability.max_search_bytes {
                    config.max_search_bytes = limit;
                }
                tools.extend(
                    FileSystemToolset::new(config)
                        .map_err(|error| error.to_string())?
                        .tools(),
                );
            }
        }
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use tokio::io::BufReader;
    use tokio::time::{Duration, Instant};

    use ariadne_config::{ConfiguredProvider, OpenAiAuthentication, ProviderSettingsStore};

    use super::{
        MAX_CODEX_MESSAGE_BYTES, OpenAiAuthenticationLock, OpenAiCredentialSelection,
        decode_optional_env, openai_account_reuses_existing_credentials, read_codex_message,
        selected_openai_codex_home, synchronize_connected_openai_provider,
        update_openai_credential_selection, verify_existing_openai_credentials_with_program,
        write_codex_message,
    };

    #[tokio::test]
    async fn openai_authentication_lock_serializes_credential_mutations() {
        let lock = OpenAiAuthenticationLock::default();
        let first = lock.acquire().await;

        assert!(
            tokio::time::timeout(Duration::from_millis(10), lock.acquire())
                .await
                .is_err()
        );

        drop(first);
        let _released = tokio::time::timeout(Duration::from_millis(50), lock.acquire())
            .await
            .expect("the credential lock should be released");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_chatgpt_reuse_rejects_an_api_key_login() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("codex");
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf '%s\\n' 'Logged in using an API key'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

        let error = verify_existing_openai_credentials_with_program(&program)
            .await
            .unwrap_err();

        assert_eq!(error, "existing ChatGPT credentials are unavailable");

        std::fs::write(
            &program,
            "#!/bin/sh\nprintf '%s\\n' 'Logged in using ChatGPT'\n",
        )
        .unwrap();
        verify_existing_openai_credentials_with_program(&program)
            .await
            .unwrap();
    }

    #[test]
    fn configured_openai_reuse_selects_the_normal_codex_account() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings =
            ProviderSettingsStore::load(directory.path().join("providers.toml")).unwrap();
        settings
            .add(ConfiguredProvider::OpenAi {
                authentication: OpenAiAuthentication::Chatgpt,
                reuse_existing: true,
            })
            .unwrap();

        assert!(openai_account_reuses_existing_credentials(&settings));
    }

    #[test]
    fn openai_credential_selection_updates_without_restarting() {
        let selection = OpenAiCredentialSelection::new(false);

        assert!(!selection.reuses_existing());
        selection.set_reuse_existing(true);
        assert!(selection.reuses_existing());
        selection.set_reuse_existing(false);
        assert!(!selection.reuses_existing());
    }

    #[test]
    fn openai_account_status_uses_the_selected_credential_home() {
        let selection = OpenAiCredentialSelection::new(false);
        let private_home = PathBuf::from("/private/ariadne/codex");

        assert_eq!(
            selected_openai_codex_home(&selection, &private_home),
            Some(private_home.clone())
        );

        selection.set_reuse_existing(true);
        assert_eq!(selected_openai_codex_home(&selection, &private_home), None);
    }

    #[test]
    fn non_openai_provider_changes_preserve_openai_credential_selection() {
        let selection = OpenAiCredentialSelection::new(true);

        update_openai_credential_selection(
            &selection,
            &ConfiguredProvider::Ollama {
                api_base: "http://127.0.0.1:11434/v1".to_owned(),
            },
        );

        assert!(selection.reuses_existing());
    }

    #[test]
    fn direct_openai_connection_switches_a_reused_provider_to_private_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings =
            ProviderSettingsStore::load(directory.path().join("providers.toml")).unwrap();
        settings
            .add(ConfiguredProvider::OpenAi {
                authentication: OpenAiAuthentication::Chatgpt,
                reuse_existing: true,
            })
            .unwrap();
        let selection = OpenAiCredentialSelection::new(true);

        synchronize_connected_openai_provider(
            &mut settings,
            &selection,
            OpenAiAuthentication::Chatgpt,
        )
        .unwrap();

        assert!(!selection.reuses_existing());
        assert!(matches!(
            settings.get("openai"),
            Some(ConfiguredProvider::OpenAi {
                authentication: OpenAiAuthentication::Chatgpt,
                reuse_existing: false,
            })
        ));
    }

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

    #[tokio::test]
    async fn codex_message_reader_rejects_an_oversized_unterminated_line() {
        let input = vec![b'x'; MAX_CODEX_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());

        let error = read_codex_message(&mut reader, Instant::now() + Duration::from_secs(1))
            .await
            .unwrap_err();

        assert_eq!(error, "Codex app-server message exceeded the size limit");
    }

    #[tokio::test]
    async fn codex_message_reader_observes_the_operation_deadline() {
        let (_writer, reader) = tokio::io::duplex(16);
        let mut reader = BufReader::new(reader);

        let error = read_codex_message(&mut reader, Instant::now() + Duration::from_millis(1))
            .await
            .unwrap_err();

        assert_eq!(error, "Codex app-server response timed out");
    }

    #[tokio::test]
    async fn codex_message_writer_rejects_an_oversized_request() {
        let (mut writer, _reader) = tokio::io::duplex(16);
        let message = serde_json::json!({"value": "x".repeat(MAX_CODEX_MESSAGE_BYTES)});

        let error = write_codex_message(
            &mut writer,
            &message,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Codex app-server request exceeded the size limit");
    }

    #[tokio::test]
    async fn codex_message_writer_observes_the_operation_deadline() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let message = serde_json::json!({"value": "blocked"});

        let error = write_codex_message(
            &mut writer,
            &message,
            Instant::now() + Duration::from_millis(1),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Codex app-server request timed out");
    }
}
