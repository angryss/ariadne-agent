use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ariadne_config::{
    ConfiguredProvider, OpenAiAuthentication, ProviderSettingsStore, secure_private_directory,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProviderChoice {
    Ollama,
    OpenAi,
}

impl ProviderChoice {
    fn id(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::OpenAi => "OpenAI",
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::Ollama => Self::OpenAi,
            Self::OpenAi => Self::Ollama,
        }
    }
}

struct ProviderUi {
    providers: Vec<ConfiguredProvider>,
    selected: usize,
    editing: bool,
    existing: bool,
    choice: ProviderChoice,
    authentication: OpenAiAuthentication,
    input: String,
    error: Option<String>,
}

impl ProviderUi {
    fn new(providers: Vec<ConfiguredProvider>) -> Self {
        Self {
            providers,
            selected: 0,
            editing: false,
            existing: false,
            choice: ProviderChoice::Ollama,
            authentication: OpenAiAuthentication::Chatgpt,
            input: String::new(),
            error: None,
        }
    }

    fn begin_add(&mut self) {
        self.existing = false;
        self.choice = if self.has("ollama") {
            ProviderChoice::OpenAi
        } else {
            ProviderChoice::Ollama
        };
        self.authentication = OpenAiAuthentication::Chatgpt;
        self.input = if self.choice == ProviderChoice::Ollama {
            "http://127.0.0.1:11434/v1".to_owned()
        } else {
            String::new()
        };
        self.error = None;
        self.editing = true;
    }

    fn begin_edit(&mut self) {
        let Some(provider) = self.providers.get(self.selected) else {
            return;
        };
        self.existing = true;
        self.error = None;
        match provider {
            ConfiguredProvider::Ollama { api_base } => {
                self.choice = ProviderChoice::Ollama;
                self.input = api_base.clone();
            }
            ConfiguredProvider::OpenAi { authentication } => {
                self.choice = ProviderChoice::OpenAi;
                self.authentication = *authentication;
                self.input.clear();
            }
        }
        self.editing = true;
    }

    fn has(&self, id: &str) -> bool {
        self.providers.iter().any(|provider| provider.id() == id)
    }

    fn refresh(&mut self, store: &ProviderSettingsStore) {
        self.providers = store.list();
        self.selected = self.selected.min(self.providers.len().saturating_sub(1));
    }
}

pub fn run(path: PathBuf) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("provider configuration requires an interactive terminal");
    }
    let mut store =
        ProviderSettingsStore::load(path).context("failed to load Ariadne provider settings")?;
    let mut ui = ProviderUi::new(store.list());
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let result = run_loop(&mut ui, &mut store, &mut stdout);
    let _ = disable_raw_mode();
    let _ = execute!(stdout, LeaveAlternateScreen);
    result
}

fn run_loop(
    ui: &mut ProviderUi,
    store: &mut ProviderSettingsStore,
    stdout: &mut io::Stdout,
) -> Result<()> {
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize provider TUI")?;
    loop {
        if !ui.editing {
            store
                .refresh()
                .context("failed to refresh provider settings")?;
            ui.refresh(store);
        }
        terminal.draw(|frame| draw(frame, ui))?;
        let Event::Key(key) = event::read().context("failed to read terminal input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if ui.editing {
            match key.code {
                KeyCode::Esc => {
                    ui.editing = false;
                    ui.input.clear();
                    ui.error = None;
                }
                KeyCode::Left | KeyCode::Right if !ui.existing => {
                    let next = ui.choice.toggle();
                    if !ui.has(next.id()) {
                        ui.choice = next;
                        ui.input = if next == ProviderChoice::Ollama {
                            "http://127.0.0.1:11434/v1".to_owned()
                        } else {
                            String::new()
                        };
                    }
                }
                KeyCode::Tab if ui.choice == ProviderChoice::OpenAi => {
                    ui.authentication = match ui.authentication {
                        OpenAiAuthentication::ApiKey => OpenAiAuthentication::Chatgpt,
                        OpenAiAuthentication::Chatgpt => OpenAiAuthentication::ApiKey,
                    };
                    ui.input.clear();
                }
                KeyCode::Backspace => {
                    ui.input.pop();
                }
                KeyCode::Char(character) => ui.input.push(character),
                KeyCode::Enter => save_provider_with_terminal_suspended(&mut terminal, ui, store)?,
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('a') if ui.providers.len() < 2 => ui.begin_add(),
            KeyCode::Char('e') | KeyCode::Enter => ui.begin_edit(),
            KeyCode::Char('d') => {
                if let Some(provider) = ui.providers.get(ui.selected) {
                    if let Err(error) = store.delete(provider.id()) {
                        ui.error = Some(error.to_string());
                    } else {
                        ui.refresh(store);
                    }
                }
            }
            KeyCode::Up => ui.selected = ui.selected.saturating_sub(1),
            KeyCode::Down => {
                ui.selected = (ui.selected + 1).min(ui.providers.len().saturating_sub(1));
            }
            _ => {}
        }
    }
    Ok(())
}

fn save_provider_with_terminal_suspended(
    terminal: &mut Terminal<CrosstermBackend<&mut io::Stdout>>,
    ui: &mut ProviderUi,
    store: &mut ProviderSettingsStore,
) -> Result<()> {
    disable_raw_mode().context("failed to suspend terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to suspend alternate screen")?;

    save_provider(ui, store);

    execute!(terminal.backend_mut(), EnterAlternateScreen)
        .context("failed to restore alternate screen")?;
    enable_raw_mode().context("failed to restore terminal raw mode")?;
    terminal
        .clear()
        .context("failed to redraw provider settings")?;
    Ok(())
}

fn save_provider(ui: &mut ProviderUi, store: &mut ProviderSettingsStore) {
    let provider = match ui.choice {
        ProviderChoice::Ollama => ConfiguredProvider::Ollama {
            api_base: ui.input.trim().to_owned(),
        },
        ProviderChoice::OpenAi => {
            if let Err(error) = authenticate_openai(ui.authentication, &ui.input) {
                ui.error = Some(error.to_string());
                ui.input.clear();
                return;
            }
            ConfiguredProvider::OpenAi {
                authentication: ui.authentication,
            }
        }
    };
    let result = if ui.existing {
        store.update(provider)
    } else {
        store.add(provider)
    };
    match result {
        Ok(()) => {
            ui.refresh(store);
            ui.editing = false;
            ui.input.clear();
            ui.error = None;
        }
        Err(error) => ui.error = Some(error.to_string()),
    }
}

fn authenticate_openai(authentication: OpenAiAuthentication, api_key: &str) -> Result<()> {
    let codex_home = std::env::var_os("ARIADNE_CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|path| path.join("ariadne").join("codex")))
        .context("Ariadne could not determine its configuration directory")?;
    let codex_home = secure_private_directory(codex_home)
        .context("failed to prepare secure OpenAI credentials")?;
    let program = std::env::var_os("ARIADNE_CODEX_PATH").unwrap_or_else(|| "codex".into());
    let mut command = Command::new(program);
    command
        .env("CODEX_HOME", codex_home)
        .arg("login")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (child, timeout) = match authentication {
        OpenAiAuthentication::Chatgpt => (
            command.stdin(Stdio::null()).spawn(),
            Duration::from_secs(300),
        ),
        OpenAiAuthentication::ApiKey => {
            if api_key.trim().is_empty() {
                bail!("OpenAI API key must not be blank");
            }
            if api_key.len() > 16 * 1024 {
                bail!("OpenAI API key is too large");
            }
            let mut child = command
                .arg("--with-api-key")
                .stdin(Stdio::piped())
                .spawn()?;
            let mut stdin = child
                .stdin
                .take()
                .context("Codex login stdin is unavailable")?;
            stdin.write_all(api_key.as_bytes())?;
            stdin.write_all(b"\n")?;
            drop(stdin);
            (Ok(child), Duration::from_secs(120))
        }
    };
    let status = wait_for_child(child.context("failed to start OpenAI sign-in")?, timeout)?;
    if !status.success() {
        bail!("OpenAI sign-in did not complete");
    }
    Ok(())
}

fn wait_for_child(mut child: Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to wait for OpenAI sign-in")?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("OpenAI sign-in timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, ui: &ProviderUi) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new("Ariadne Settings — Providers")
            .style(
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    if ui.editing {
        let authentication = match ui.authentication {
            OpenAiAuthentication::ApiKey => "API key",
            OpenAiAuthentication::Chatgpt => "ChatGPT subscription",
        };
        let displayed = if ui.choice == ProviderChoice::OpenAi
            && ui.authentication == OpenAiAuthentication::ApiKey
        {
            "•".repeat(ui.input.chars().count())
        } else {
            ui.input.clone()
        };
        let text = Text::from(vec![
            Line::from(format!("Provider: {}  (←/→ to change)", ui.choice.title())),
            Line::from(if ui.choice == ProviderChoice::OpenAi {
                format!("Authentication: {authentication}  (Tab to change)")
            } else {
                "Ollama API base URL:".to_owned()
            }),
            Line::from(displayed),
            Line::from(""),
            Line::from("Enter save • Esc cancel"),
        ]);
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::default().title("Provider").borders(Borders::ALL))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    } else if ui.providers.is_empty() {
        frame.render_widget(
            Paragraph::new("No providers configured.\n\nPress a to add Ollama or OpenAI.")
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );
    } else {
        let items = ui.providers.iter().map(|provider| {
            let detail = match provider {
                ConfiguredProvider::Ollama { api_base } => api_base.as_str(),
                ConfiguredProvider::OpenAi {
                    authentication: OpenAiAuthentication::ApiKey,
                } => "API key",
                ConfiguredProvider::OpenAi {
                    authentication: OpenAiAuthentication::Chatgpt,
                } => "ChatGPT subscription",
            };
            ListItem::new(format!("{}  {detail}", provider_title(provider)))
        });
        let mut state = ListState::default().with_selected(Some(ui.selected));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
            &mut state,
        );
    }
    let status = ui
        .error
        .as_deref()
        .unwrap_or("a add • e/Enter edit • d delete • ↑/↓ select • q quit");
    frame.render_widget(
        Paragraph::new(status).style(if ui.error.is_some() {
            Style::default().fg(Color::LightRed)
        } else {
            Style::default().fg(Color::DarkGray)
        }),
        chunks[2],
    );
}

fn provider_title(provider: &ConfiguredProvider) -> &'static str {
    match provider {
        ConfiguredProvider::Ollama { .. } => "Ollama",
        ConfiguredProvider::OpenAi { .. } => "OpenAI",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    use super::{ProviderUi, wait_for_child};
    use ariadne_config::ConfiguredProvider;

    #[cfg(unix)]
    #[test]
    fn openai_login_timeout_kills_a_hung_child() {
        let child = Command::new("sh")
            .args(["-c", "sleep 5"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();

        let error = wait_for_child(child, Duration::from_millis(10)).unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn provider_ui_starts_with_an_empty_list_and_can_prepare_both_provider_forms() {
        let mut ui = ProviderUi::new(Vec::new());
        assert!(ui.providers.is_empty());
        ui.begin_add();
        assert_eq!(ui.choice.id(), "ollama");

        ui.providers.push(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        });
        ui.begin_add();
        assert_eq!(ui.choice.id(), "openai");
    }
}
