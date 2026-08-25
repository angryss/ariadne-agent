use std::io::{self, Stdout};

use anyhow::{Context, Result};
use ariadne_core::{AgentProfiles, Message};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageKind {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DisplayMessage {
    kind: MessageKind,
    content: String,
}

struct ChatUi {
    messages: Vec<DisplayMessage>,
    input: String,
    cursor: usize,
    profile: String,
    model: String,
    busy: bool,
    scroll_from_bottom: u16,
}

impl ChatUi {
    fn new(profile: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            profile: profile.into(),
            model: model.into(),
            busy: false,
            scroll_from_bottom: 0,
        }
    }

    #[cfg(test)]
    fn messages(&self) -> &[DisplayMessage] {
        &self.messages
    }

    fn push_message(&mut self, kind: MessageKind, content: impl Into<String>) {
        self.messages.push(DisplayMessage {
            kind,
            content: content.into(),
        });
        self.scroll_from_bottom = 0;
    }

    fn take_submission(&mut self) -> Option<String> {
        let prompt = self.input.trim().to_owned();
        if prompt.is_empty() {
            return None;
        }
        self.input.clear();
        self.cursor = 0;
        self.push_message(MessageKind::User, prompt.clone());
        Some(prompt)
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.insert(self.cursor, character);
                self.cursor += character.len_utf8();
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.input[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(index, _)| index)
                    .unwrap_or(0);
                self.input.drain(previous..self.cursor);
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.input.len() => {
                let next = self.input[self.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(index, _)| self.cursor + index)
                    .unwrap_or(self.input.len());
                self.input.drain(self.cursor..next);
            }
            KeyCode::Left => {
                self.cursor = self.input[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(index, _)| index)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                self.cursor = self.input[self.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(index, _)| self.cursor + index)
                    .unwrap_or(self.input.len());
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            _ => {}
        }
    }
}

struct ChatLayout {
    transcript: Rect,
    composer: Rect,
    status: Rect,
}

fn chat_layout(area: Rect) -> ChatLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(area);
    ChatLayout {
        transcript: chunks[0],
        composer: chunks[1],
        status: chunks[2],
    }
}

fn transcript_text(ui: &ChatUi) -> Text<'static> {
    let mut lines = vec![Line::styled(
        "Ariadne",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if ui.messages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Ask a question or describe a task to begin.",
            Style::default().fg(Color::DarkGray),
        ));
    }
    for message in &ui.messages {
        lines.push(Line::from(""));
        let (label, style) = match message.kind {
            MessageKind::User => (
                "❯ You",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            MessageKind::Assistant => (
                "● Ariadne",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            MessageKind::Error => (
                "! Error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        };
        lines.push(Line::styled(label, style));
        lines.extend(
            message
                .content
                .lines()
                .map(|line| Line::from(line.to_owned())),
        );
    }
    Text::from(lines)
}

fn render(frame: &mut Frame<'_>, ui: &ChatUi) {
    let layout = chat_layout(frame.area());
    let text = transcript_text(ui);
    let transcript = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    let total_lines = transcript.line_count(layout.transcript.width) as u16;
    let maximum_scroll = total_lines.saturating_sub(layout.transcript.height);
    let scroll = maximum_scroll.saturating_sub(ui.scroll_from_bottom);
    frame.render_widget(transcript.scroll((scroll, 0)), layout.transcript);

    let composer = Paragraph::new(Line::from(vec![
        Span::styled(
            "› ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(&ui.input),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(composer, layout.composer);

    let state = if ui.busy { "Thinking…" } else { "Ready" };
    let status = Line::from(vec![
        Span::styled(
            format!(" {state} "),
            Style::default().fg(if ui.busy { Color::Yellow } else { Color::Green }),
        ),
        Span::styled(
            format!("{} · {}", ui.profile, ui.model),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "  Enter send · PgUp/PgDn scroll · Ctrl-C exit",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(status), layout.status);

    if !ui.busy && layout.composer.width > 4 {
        let cursor_column = ui.input[..ui.cursor].chars().count() as u16;
        let x = layout
            .composer
            .x
            .saturating_add(2)
            .saturating_add(cursor_column)
            .min(layout.composer.right().saturating_sub(2));
        frame.set_cursor_position((x, layout.composer.y.saturating_add(1)));
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter terminal screen");
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                return Err(error).context("failed to initialize terminal UI");
            }
        };
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub async fn run(profiles: &AgentProfiles, profile: &str, model: &str) -> Result<()> {
    let mut session = TerminalSession::enter()?;
    let mut ui = ChatUi::new(profile, model);
    let mut history = Vec::<Message>::new();

    loop {
        session
            .terminal
            .draw(|frame| render(frame, &ui))
            .context("failed to draw terminal UI")?;
        let Event::Key(key) = event::read().context("failed to read terminal input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            break;
        }
        match key.code {
            KeyCode::PageUp => ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_add(5),
            KeyCode::PageDown => ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_sub(5),
            KeyCode::Enter => {
                let Some(prompt) = ui.take_submission() else {
                    continue;
                };
                if matches!(prompt.as_str(), ":quit" | ":exit") {
                    break;
                }
                ui.busy = true;
                session
                    .terminal
                    .draw(|frame| render(frame, &ui))
                    .context("failed to draw terminal UI")?;
                match profiles.respond(Some(profile), &history, &prompt).await {
                    Ok(message) => {
                        ui.push_message(
                            MessageKind::Assistant,
                            super::sanitize_terminal_text(&message.content),
                        );
                        history.push(Message::user(prompt));
                        history.push(message);
                    }
                    Err(error) => ui.push_message(
                        MessageKind::Error,
                        super::sanitize_terminal_text(&error.to_string()),
                    ),
                }
                ui.busy = false;
            }
            _ if !ui.busy => ui.handle_edit_key(key),
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    use super::{ChatUi, MessageKind, chat_layout, render};

    #[test]
    fn layout_keeps_the_composer_and_status_at_the_bottom() {
        let area = Rect::new(0, 0, 100, 30);

        let layout = chat_layout(area);

        assert_eq!(layout.transcript, Rect::new(0, 0, 100, 25));
        assert_eq!(layout.composer, Rect::new(0, 25, 100, 4));
        assert_eq!(layout.status, Rect::new(0, 29, 100, 1));
    }

    #[test]
    fn render_keeps_the_latest_chat_and_input_visible() {
        let backend = TestBackend::new(48, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut ui = ChatUi::new("local", "test-model");
        for index in 0..12 {
            ui.push_message(MessageKind::Assistant, format!("old response {index}"));
        }
        ui.push_message(MessageKind::User, "latest question");
        ui.input = "next prompt".to_owned();

        terminal.draw(|frame| render(frame, &ui)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("latest question"), "{screen}");
        assert!(screen.contains("› next prompt"), "{screen}");
        assert!(screen.contains("local · test-model"), "{screen}");
        assert!(!screen.contains("old response 0"), "{screen}");
    }

    #[test]
    fn editor_supports_cursor_movement_and_insertion() {
        let mut ui = ChatUi::new("local", "test-model");
        ui.input = "helo".to_owned();
        ui.cursor = 2;

        ui.handle_edit_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        ui.handle_edit_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        ui.handle_edit_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));

        assert_eq!(ui.input, "hello!");
        assert_eq!(ui.cursor, ui.input.len());
    }

    #[test]
    fn submitting_moves_the_prompt_into_the_transcript() {
        let mut ui = ChatUi::new("local", "test-model");
        ui.input = "Explain this code".to_owned();
        ui.cursor = ui.input.len();

        let prompt = ui.take_submission().unwrap();

        assert_eq!(prompt, "Explain this code");
        assert!(ui.input.is_empty());
        assert_eq!(ui.cursor, 0);
        assert_eq!(ui.messages().last().unwrap().content, "Explain this code");
        assert_eq!(ui.messages().last().unwrap().kind, MessageKind::User);
    }
}
