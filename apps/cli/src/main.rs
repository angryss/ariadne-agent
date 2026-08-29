use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::{io, io::BufRead, io::IsTerminal, io::Read, io::Write};

use anyhow::{Context, Result, ensure};
use ariadne_config::{
    ProfileCatalog, ProviderKind, ProviderSettingsStore, ResolvedCapability, ResolvedProfile,
};
use ariadne_core::{Agent, AgentProfiles, ModelProvider, Tool};
use ariadne_provider_anthropic::{AnthropicMessagesProvider, ClaudeCodeProvider};
use ariadne_provider_openai::OpenAiCompatibleProvider;
use ariadne_tools_command::{CommandConfig, CommandTool};
use ariadne_tools_filesystem::{FileSystemConfig, FileSystemToolset};
use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

mod chat_ui;
mod provider_ui;

#[derive(Parser)]
#[command(name = "ariadne", version, about = "A local-first AI agent")]
struct Cli {
    /// TOML configuration file. Uses the platform default when omitted.
    #[arg(long, env = "ARIADNE_CONFIG", global = true)]
    config: Option<PathBuf>,
    /// Provider settings file. Uses the platform default when omitted.
    #[arg(long, env = "ARIADNE_PROVIDER_CONFIG", global = true)]
    provider_config: Option<PathBuf>,
    /// Open the interactive provider settings interface.
    #[arg(long, global = true)]
    configure_providers: bool,
    /// Profile to use as the process default.
    #[arg(long, env = "ARIADNE_PROFILE", global = true)]
    profile: Option<String>,
    /// Base URL for an OpenAI-compatible API, including any `/v1` prefix.
    #[arg(long, env = "ARIADNE_API_BASE", global = true)]
    api_base: Option<String>,
    /// Model identifier understood by the provider.
    #[arg(long, env = "ARIADNE_MODEL", global = true)]
    model: Option<String>,
    /// Optional API key. Prefer the environment variable to shell history.
    #[arg(long, env = "ARIADNE_API_KEY", global = true, hide_env_values = true)]
    api_key: Option<String>,
    /// Trusted system instruction prepended to every request.
    #[arg(long, env = "ARIADNE_SYSTEM_PROMPT", global = true)]
    system_prompt: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start an interactive terminal conversation.
    Chat,
    /// Run one prompt and exit for scripts, cron, and automation.
    Run {
        /// Prompt text. Reads stdin when omitted.
        #[arg(long)]
        prompt: Option<String>,
        /// Response encoding written to stdout.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        output: OutputFormat,
    },
    /// Run the HTTP API and optional web application.
    Serve {
        /// Address to listen on. Loopback is the secure default.
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
        /// Directory containing a built web application.
        #[arg(long)]
        web_dir: Option<PathBuf>,
    },
    /// List configured profiles without contacting model providers.
    Profiles {
        /// Profile-list encoding written to stdout.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        output: OutputFormat,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Default)]
struct ProfileOverrides {
    api_base: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    system_prompt: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let provider_config = match cli.provider_config.clone() {
        Some(path) => path,
        None => ProviderSettingsStore::default_path()
            .context("failed to locate Ariadne provider settings")?,
    };
    if cli.configure_providers {
        return provider_ui::run(provider_config);
    }
    let catalog = match &cli.config {
        Some(path) => ProfileCatalog::load(path)
            .with_context(|| format!("failed to load configuration from {}", path.display()))?,
        None => ProfileCatalog::load_default().context("failed to load Ariadne configuration")?,
    };
    let default_profile = cli
        .profile
        .clone()
        .unwrap_or_else(|| catalog.default_profile().to_owned());
    let command = cli.command.unwrap_or(Command::Chat);
    if let Command::Profiles { output } = command {
        return list_profiles(&catalog, &default_profile, cli.model.as_deref(), output);
    }
    let include_all_profiles = matches!(&command, Command::Serve { .. });
    let profiles = configured_profiles(
        &catalog,
        &default_profile,
        ProfileOverrides {
            api_base: cli.api_base,
            model: cli.model,
            api_key: cli.api_key,
            system_prompt: cli.system_prompt,
        },
        include_all_profiles,
    )?;

    match command {
        Command::Run { prompt, output } => {
            run_once(&profiles, &default_profile, prompt, output).await
        }
        Command::Chat => chat(&profiles, &default_profile).await,
        Command::Serve { bind, web_dir } => serve(profiles, bind, web_dir, provider_config).await,
        Command::Profiles { .. } => unreachable!("profiles returned before provider configuration"),
    }
}

fn list_profiles(
    catalog: &ProfileCatalog,
    default_profile: &str,
    model_override: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    if let Some(model) = model_override {
        ensure!(!model.trim().is_empty(), "provider model must not be blank");
    }
    catalog
        .resolve(default_profile)
        .with_context(|| format!("failed to select profile `{default_profile}`"))?;
    let profiles = catalog
        .resolve_all()?
        .into_iter()
        .map(|profile| {
            let mut profile = profile.profile;
            if profile.name == default_profile
                && let Some(model) = model_override
            {
                profile.model = model.to_owned();
            }
            profile
        })
        .collect::<Vec<_>>();

    match output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "default_profile": default_profile,
                "profiles": profiles,
            }))?
        ),
        OutputFormat::Text => {
            for profile in profiles {
                let marker = if profile.name == default_profile {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{marker} {}\tprovider={}\tmodel={}\tskills={}\tmcp_servers={}\tcapabilities={}",
                    profile.name,
                    profile.provider,
                    profile.model,
                    profile.active_skills.join(","),
                    profile.mcp_servers.join(","),
                    profile.capabilities.join(",")
                );
            }
        }
    }
    Ok(())
}

fn configured_profiles(
    catalog: &ProfileCatalog,
    default_profile: &str,
    overrides: ProfileOverrides,
    include_all_profiles: bool,
) -> Result<AgentProfiles> {
    let selected = catalog
        .resolve(default_profile)
        .with_context(|| format!("failed to select profile `{default_profile}`"))?;
    let resolved = if include_all_profiles {
        catalog.resolve_all()?
    } else {
        vec![selected]
    };
    let mut configured = Vec::new();
    for mut profile in resolved {
        let api_key_override = if profile.profile.name == default_profile {
            if let Some(api_base) = &overrides.api_base {
                profile.api_base.clone_from(api_base);
            }
            if let Some(model) = &overrides.model {
                profile.profile.model.clone_from(model);
            }
            if let Some(system_prompt) = &overrides.system_prompt {
                profile.system_prompt.clone_from(system_prompt);
            }
            overrides.api_key.clone()
        } else {
            None
        };
        let agent = configured_agent(&profile, api_key_override)?;
        configured.push((profile.profile, agent));
    }

    AgentProfiles::new(default_profile, configured).context("invalid profile catalog")
}

fn configured_agent(profile: &ResolvedProfile, api_key_override: Option<String>) -> Result<Agent> {
    let api_key = match api_key_override {
        Some(api_key) => Some(api_key),
        None => profile
            .api_key_env
            .as_deref()
            .map(|name| {
                env::var(name).with_context(|| {
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
                .with_context(|| {
                    format!(
                        "invalid model provider configuration for profile `{}`",
                        profile.profile.name
                    )
                })?,
        ),
        ProviderKind::AnthropicMessages => Arc::new(
            AnthropicMessagesProvider::with_base_url(
                &profile.api_base,
                &profile.profile.model,
                api_key.ok_or_else(|| {
                    anyhow::anyhow!(
                        "profile `{}` requires an Anthropic API key",
                        profile.profile.name
                    )
                })?,
            )
            .with_context(|| {
                format!(
                    "invalid Anthropic provider configuration for profile `{}`",
                    profile.profile.name
                )
            })?,
        ),
        ProviderKind::ClaudeSubscription => Arc::new(ClaudeCodeProvider::new(
            &profile.claude_program,
            &profile.profile.model,
        )),
    };

    if profile.provider_kind == ProviderKind::ClaudeSubscription && !profile.capabilities.is_empty()
    {
        anyhow::bail!("Claude subscription profiles do not support Ariadne capabilities");
    }
    let tools = configured_tools(profile)?;
    if tools.is_empty() {
        Ok(Agent::new(provider, profile.system_prompt.clone()))
    } else {
        Agent::with_tools(provider, profile.system_prompt.clone(), tools)
            .context("invalid profile tool configuration")
    }
}

fn configured_tools(profile: &ResolvedProfile) -> Result<Vec<Arc<dyn Tool>>> {
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
                    .context("invalid command capability")?,
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
                        .context("invalid filesystem capability")?
                        .tools(),
                );
            }
        }
    }
    Ok(tools)
}

async fn chat(profiles: &AgentProfiles, profile: &str) -> Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let model = profiles
            .profiles()
            .into_iter()
            .find(|candidate| candidate.name == profile)
            .map(|candidate| candidate.model)
            .with_context(|| format!("profile `{profile}` is not configured"))?;
        return chat_ui::run(profiles, profile, &model).await;
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut history = Vec::new();
    let mut line = String::new();

    println!("Ariadne interactive mode. Type /quit to exit.");
    loop {
        print!("you> ");
        io::stdout().flush().context("failed to flush stdout")?;
        line.clear();
        if input.read_line(&mut line).context("failed to read stdin")? == 0 {
            break;
        }

        let prompt = line.trim_end();
        if matches!(prompt, "/quit" | "/exit") {
            break;
        }
        if prompt.trim().is_empty() {
            continue;
        }

        let message = profiles
            .respond(Some(profile), &history, prompt)
            .await
            .map_err(sanitize_agent_error)?;
        println!("ariadne> {}", sanitize_terminal_text(&message.content));
        history.push(ariadne_core::Message::user(prompt));
        history.push(message);
    }

    Ok(())
}

async fn serve(
    profiles: AgentProfiles,
    bind: SocketAddr,
    web_dir: Option<PathBuf>,
    provider_config: PathBuf,
) -> Result<()> {
    let provider_settings = ProviderSettingsStore::load(provider_config)
        .context("failed to load Ariadne provider settings")?;
    let app = match web_dir {
        Some(web_dir) => {
            ensure!(
                web_dir.join("index.html").is_file(),
                "web directory does not contain index.html: {}",
                web_dir.display()
            );
            ariadne_server::router_with_profiles_provider_settings_and_web(
                profiles,
                provider_settings,
                web_dir,
            )
        }
        None => {
            ariadne_server::router_with_profiles_and_provider_settings(profiles, provider_settings)
        }
    };
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind Ariadne server to {bind}"))?;
    tracing::info!(address = %bind, "Ariadne server started");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("Ariadne server failed")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(terminate) => terminate,
            Err(error) => {
                tracing::error!(%error, "failed to install SIGTERM handler");
                wait_for_ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            () = wait_for_ctrl_c() => {}
            signal = terminate.recv() => {
                if signal.is_none() {
                    tracing::error!("SIGTERM handler closed before receiving a signal");
                }
            }
        }
    }

    #[cfg(not(unix))]
    wait_for_ctrl_c().await;
}

async fn wait_for_ctrl_c() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}

async fn run_once(
    profiles: &AgentProfiles,
    profile: &str,
    prompt: Option<String>,
    output: OutputFormat,
) -> Result<()> {
    let prompt = match prompt {
        Some(prompt) => prompt,
        None => {
            let mut prompt = String::new();
            io::stdin()
                .read_to_string(&mut prompt)
                .context("failed to read prompt from stdin")?;
            prompt.trim_end().to_owned()
        }
    };
    let message = profiles
        .respond(Some(profile), &[], &prompt)
        .await
        .map_err(sanitize_agent_error)?;

    match output {
        OutputFormat::Text => println!("{}", sanitize_terminal_text(&message.content)),
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(&ariadne_server::RespondResponse { message })?
        ),
    }
    Ok(())
}

fn sanitize_agent_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::Error::msg(sanitize_terminal_text(&error.to_string()))
}

fn sanitize_terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| matches!(character, '\n' | '\t') || !character.is_control())
        .collect()
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::sanitize_terminal_text;

    #[test]
    fn terminal_sanitizer_removes_controls_but_preserves_newlines_and_tabs() {
        let malicious = "safe\u{1b}[2J\u{1b}]0;owned\u{7}\r\u{8}\u{9b}31m\n\ttext";

        assert_eq!(
            sanitize_terminal_text(malicious),
            "safe[2J]0;owned31m\n\ttext"
        );
    }
}
