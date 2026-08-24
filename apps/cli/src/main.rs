use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::{io, io::BufRead, io::Read, io::Write};

use anyhow::{Context, Result, ensure};
use ariadne_core::{Agent, ModelProvider};
use ariadne_provider_openai::OpenAiCompatibleProvider;
use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "ariadne", version, about = "A local-first AI agent")]
struct Cli {
    /// Base URL for an OpenAI-compatible API, including any `/v1` prefix.
    #[arg(
        long,
        env = "ARIADNE_API_BASE",
        global = true,
        default_value = "http://127.0.0.1:11434/v1"
    )]
    api_base: String,
    /// Model identifier understood by the provider.
    #[arg(long, env = "ARIADNE_MODEL", global = true, default_value = "qwen3:8b")]
    model: String,
    /// Optional API key. Prefer the environment variable to shell history.
    #[arg(long, env = "ARIADNE_API_KEY", global = true, hide_env_values = true)]
    api_key: Option<String>,
    /// Trusted system instruction prepended to every request.
    #[arg(
        long,
        env = "ARIADNE_SYSTEM_PROMPT",
        global = true,
        default_value = "You are Ariadne, a careful and capable AI software agent."
    )]
    system_prompt: String,
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
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let provider = OpenAiCompatibleProvider::new(cli.api_base, cli.model, cli.api_key)
        .context("invalid model provider configuration")?;
    let agent = Agent::new(
        Arc::new(provider) as Arc<dyn ModelProvider>,
        cli.system_prompt,
    );

    match cli.command.unwrap_or(Command::Chat) {
        Command::Run { prompt, output } => run_once(&agent, prompt, output).await,
        Command::Chat => chat(&agent).await,
        Command::Serve { bind, web_dir } => serve(agent, bind, web_dir).await,
    }
}

async fn chat(agent: &Agent) -> Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut history = Vec::new();
    let mut line = String::new();

    println!("Ariadne interactive mode. Type :quit to exit.");
    loop {
        print!("you> ");
        io::stdout().flush().context("failed to flush stdout")?;
        line.clear();
        if input.read_line(&mut line).context("failed to read stdin")? == 0 {
            break;
        }

        let prompt = line.trim_end();
        if prompt == ":quit" || prompt == ":exit" {
            break;
        }
        if prompt.trim().is_empty() {
            continue;
        }

        let message = agent
            .respond(&history, prompt)
            .await
            .map_err(sanitize_agent_error)?;
        println!("ariadne> {}", sanitize_terminal_text(&message.content));
        history.push(ariadne_core::Message::user(prompt));
        history.push(message);
    }

    Ok(())
}

async fn serve(agent: Agent, bind: SocketAddr, web_dir: Option<PathBuf>) -> Result<()> {
    let app = match web_dir {
        Some(web_dir) => {
            ensure!(
                web_dir.join("index.html").is_file(),
                "web directory does not contain index.html: {}",
                web_dir.display()
            );
            ariadne_server::router_with_web(agent, web_dir)
        }
        None => ariadne_server::router(agent),
    };
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind Ariadne server to {bind}"))?;
    tracing::info!(address = %bind, "Ariadne server started");
    axum::serve(listener, app)
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

async fn run_once(agent: &Agent, prompt: Option<String>, output: OutputFormat) -> Result<()> {
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
    let message = agent
        .respond(&[], &prompt)
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

fn sanitize_agent_error(error: ariadne_core::AgentError) -> anyhow::Error {
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
