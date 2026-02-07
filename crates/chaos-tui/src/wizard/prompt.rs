use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::{WizardScreen, WizardState, WizardTransition};
use crate::theme;

/// 0 = prompt textarea, 1 = duration input
const FIELD_PROMPT: usize = 0;
const FIELD_DURATION: usize = 1;

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // title
            Constraint::Length(1),  // subtitle
            Constraint::Min(6),    // prompt textarea
            Constraint::Length(3), // duration input
            Constraint::Length(2),  // help
        ])
        .split(area);

    let title = Paragraph::new(" Step 3/4: Describe Your Chaos Experiment")
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(
        " Describe what chaos you want to create — include target details (DB URL, namespace, host)",
    )
    .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    // Prompt text area
    let prompt_focused = state.target_field_index == FIELD_PROMPT;
    let prompt_border = if prompt_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Chaos Prompt ")
        .borders(Borders::ALL)
        .border_style(prompt_border);

    let text = if state.prompt_input.content.is_empty() {
        Text::styled(
            "e.g., Stress test postgres://user:pass@localhost:5432/mydb with heavy inserts...",
            theme::dim_style(),
        )
    } else {
        Text::styled(&state.prompt_input.content, theme::normal_style())
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[2]);

    // Show cursor in prompt
    if prompt_focused && !state.prompt_input.content.is_empty() {
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

    // Duration input
    let duration_focused = state.target_field_index == FIELD_DURATION;
    let dur_border = if duration_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let dur_block = Block::default()
        .title(" Duration (e.g. 5m, 30s, 1h) ")
        .borders(Borders::ALL)
        .border_style(dur_border);

    let dur_paragraph = Paragraph::new(state.duration_input.content.as_str()).block(dur_block);
    dur_paragraph.render(chunks[3], frame.buffer_mut());

    // Show cursor in duration field
    if duration_focused && chunks[3].width > 2 && chunks[3].height > 0 {
        let cursor_x = chunks[3].x
            + 1
            + (state.duration_input.cursor_pos as u16).min(chunks[3].width.saturating_sub(3));
        let cursor_y = chunks[3].y + 1;
        if let Some(cell) = frame.buffer_mut().cell_mut(Position::new(cursor_x, cursor_y)) {
            cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
        }
    }

    let help = Paragraph::new(" [Tab] Switch field  [Ctrl+D] Submit  [Enter] New line (prompt)  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[4]);
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

    // Tab to switch between prompt and duration
    if key.code == KeyCode::Tab {
        state.target_field_index = if state.target_field_index == FIELD_PROMPT {
            FIELD_DURATION
        } else {
            FIELD_PROMPT
        };
        return WizardTransition::Stay;
    }

    if key.code == KeyCode::BackTab {
        state.target_field_index = if state.target_field_index == FIELD_PROMPT {
            FIELD_DURATION
        } else {
            FIELD_PROMPT
        };
        return WizardTransition::Stay;
    }

    // Route input to the active field
    if state.target_field_index == FIELD_DURATION {
        // For duration field, Enter submits (same as Ctrl+D)
        if key.code == KeyCode::Enter {
            if state.prompt_input.content.trim().is_empty() {
                state.error_message = Some("Prompt cannot be empty".to_string());
                return WizardTransition::Stay;
            }
            state.error_message = None;
            state.screen = WizardScreen::Review;
            return WizardTransition::Next(WizardScreen::Review);
        }
        state.duration_input.handle_key(key);
    } else {
        // Prompt field — Enter adds newline (handled by TextInput)
        state.prompt_input.handle_key(key);
    }
    WizardTransition::Stay
}
