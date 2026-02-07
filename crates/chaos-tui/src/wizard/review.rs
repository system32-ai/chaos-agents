use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::{WizardState, WizardTransition};
use crate::theme;

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(" Step 4/4: Review & Confirm")
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle =
        Paragraph::new(" Review your settings before starting the experiment").style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    // Build summary
    let provider = state
        .selected_provider
        .as_deref()
        .unwrap_or("unknown");

    let model = if state.model_input.content.is_empty() {
        match provider {
            "anthropic" => "claude-sonnet-4-5-20250929",
            "openai" => "gpt-4o",
            "ollama" => "llama3.1",
            _ => "unknown",
        }
    } else {
        &state.model_input.content
    };

    let duration = if state.duration_input.content.trim().is_empty() {
        "5m"
    } else {
        state.duration_input.content.trim()
    };

    let prompt_preview = if state.prompt_input.content.len() > 200 {
        format!("{}...", &state.prompt_input.content[..200])
    } else {
        state.prompt_input.content.clone()
    };

    let summary = vec![
        Line::from(vec![
            Span::styled("  Provider: ", Style::default().fg(Color::Cyan)),
            Span::styled(capitalize(provider), theme::normal_style()),
        ]),
        Line::from(vec![
            Span::styled("  Model:    ", Style::default().fg(Color::Cyan)),
            Span::styled(model, theme::normal_style()),
        ]),
        Line::from(vec![
            Span::styled("  Max Turns:", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!(" {}", if state.max_turns_input.content.is_empty() { "10" } else { &state.max_turns_input.content }),
                theme::normal_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Duration: ", Style::default().fg(Color::Cyan)),
            Span::styled(duration, theme::normal_style()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Prompt:", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled(format!("  {prompt_preview}"), theme::normal_style()),
        ]),
    ];

    let block = Block::default()
        .title(" Configuration Summary ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(summary).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[2]);

    // Confirm button
    let confirm = Paragraph::new("  [Enter] Start Experiment    [Esc] Go Back")
        .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        );
    frame.render_widget(confirm, chunks[3]);

    let help =
        Paragraph::new(" Press Enter to start execution or Esc to go back").style(theme::dim_style());
    frame.render_widget(help, chunks[4]);
}

pub fn handle_key(_state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    match key.code {
        KeyCode::Enter => WizardTransition::StartExecution,
        _ => WizardTransition::Stay,
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}
