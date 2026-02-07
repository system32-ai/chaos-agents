use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::{WizardScreen, WizardState, WizardTransition};
use crate::theme;

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(" Step 5/6: Describe Your Chaos Experiment")
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(
        " Describe what chaos you want to create in natural language",
    )
    .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    // Prompt text area
    let block = Block::default()
        .title(" Chaos Prompt ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let text = if state.prompt_input.content.is_empty() {
        Text::styled(
            "e.g., Kill a random pod in the default namespace and stress test CPU...",
            theme::dim_style(),
        )
    } else {
        Text::styled(&state.prompt_input.content, theme::normal_style())
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[2]);

    // Show cursor
    if !state.prompt_input.content.is_empty() {
        let inner = chunks[2].inner(Margin::new(1, 1));
        let lines: Vec<&str> = state.prompt_input.content[..state.prompt_input.cursor_pos]
            .split('\n')
            .collect();
        let cursor_y = inner.y + (lines.len() as u16).saturating_sub(1);
        let cursor_x = inner.x + lines.last().map(|l| l.len() as u16).unwrap_or(0);
        if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
            if let Some(cell) =
                frame.buffer_mut().cell_mut(Position::new(cursor_x, cursor_y))
            {
                cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
            }
        }
    }

    let help = Paragraph::new(" [Ctrl+D] Submit  [Enter] New line  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[3]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    // Ctrl+D to submit
    if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if state.prompt_input.content.trim().is_empty() {
            state.error_message = Some("Prompt cannot be empty".to_string());
            return WizardTransition::Stay;
        }
        state.error_message = None;
        state.screen = WizardScreen::Review;
        return WizardTransition::Next(WizardScreen::Review);
    }

    // Handle text input
    state.prompt_input.handle_key(key);
    WizardTransition::Stay
}
