use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{WizardScreen, WizardState, WizardTransition};
use crate::theme;

/// Check if the selected provider's API key is available from an environment variable.
fn env_api_key(provider: &str) -> Option<String> {
    match provider {
        "anthropic" => std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()),
        "openai" => std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty()),
        _ => None,
    }
}

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let provider = state
        .selected_provider
        .as_deref()
        .unwrap_or("unknown");

    let has_env_key = env_api_key(provider).is_some();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // 0: title
            Constraint::Length(1),  // 1: subtitle
            Constraint::Length(3),  // 2: field 1
            Constraint::Length(3),  // 3: field 2
            Constraint::Length(3),  // 4: field 3
            Constraint::Length(3),  // 5: max turns
            Constraint::Min(1),    // 6: error
            Constraint::Length(2),  // 7: help
        ])
        .split(area);

    let title = Paragraph::new(format!(" Step 2/4: Configure {}", capitalize(provider)))
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(" Set your API credentials and model preferences")
        .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    let max_turns_idx = match provider {
        "anthropic" => {
            if has_env_key {
                // Show env key detected notice instead of input
                let env_notice = Paragraph::new(" API Key: detected from ANTHROPIC_API_KEY")
                    .style(Style::default().fg(Color::Green))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
                frame.render_widget(env_notice, chunks[2]);

                // Model is field 0
                let model = input_snapshot(&state.model_input, state.provider_field_index == 0);
                model.render(chunks[3], frame.buffer_mut());

                1 // max_turns is field index 1
            } else {
                // API Key
                let api_key = input_snapshot(&state.api_key_input, state.provider_field_index == 0);
                api_key.render(chunks[2], frame.buffer_mut());

                // Model
                let model = input_snapshot(&state.model_input, state.provider_field_index == 1);
                model.render(chunks[3], frame.buffer_mut());

                2 // max_turns is field index 2
            }
        }
        "openai" => {
            if has_env_key {
                // Show env key detected notice instead of input
                let env_notice = Paragraph::new(" API Key: detected from OPENAI_API_KEY")
                    .style(Style::default().fg(Color::Green))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
                frame.render_widget(env_notice, chunks[2]);

                // Model is field 0
                let model = input_snapshot(&state.model_input, state.provider_field_index == 0);
                model.render(chunks[3], frame.buffer_mut());

                // Base URL is field 1
                let base_url = input_snapshot(&state.base_url_input, state.provider_field_index == 1);
                base_url.render(chunks[4], frame.buffer_mut());

                2 // max_turns is field index 2
            } else {
                // API Key
                let api_key = input_snapshot(&state.api_key_input, state.provider_field_index == 0);
                api_key.render(chunks[2], frame.buffer_mut());

                // Model
                let model = input_snapshot(&state.model_input, state.provider_field_index == 1);
                model.render(chunks[3], frame.buffer_mut());

                // Base URL (optional)
                let base_url =
                    input_snapshot(&state.base_url_input, state.provider_field_index == 2);
                base_url.render(chunks[4], frame.buffer_mut());

                3 // max_turns is field index 3
            }
        }
        "ollama" => {
            // Base URL
            let base_url =
                input_snapshot(&state.base_url_input, state.provider_field_index == 0);
            base_url.render(chunks[2], frame.buffer_mut());

            // Model
            let model = input_snapshot(&state.model_input, state.provider_field_index == 1);
            model.render(chunks[3], frame.buffer_mut());

            2 // max_turns is field index 2
        }
        _ => 2,
    };

    // Max Turns
    let max_turns = input_snapshot(&state.max_turns_input, state.provider_field_index == max_turns_idx);
    max_turns.render(chunks[5], frame.buffer_mut());

    // Error message
    if let Some(ref err) = state.error_message {
        let error = Paragraph::new(format!(" Error: {err}")).style(theme::error_style());
        frame.render_widget(error, chunks[6]);
    }

    let help = Paragraph::new(" [Tab] Next field  [Enter] Continue  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[7]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    let provider = state
        .selected_provider
        .as_deref()
        .unwrap_or("unknown")
        .to_string();

    let has_env_key = env_api_key(&provider).is_some();

    let max_fields = match provider.as_str() {
        "anthropic" => if has_env_key { 2 } else { 3 },  // skip api_key when from env
        "openai" => if has_env_key { 3 } else { 4 },     // skip api_key when from env
        "ollama" => 3,     // base_url, model, max_turns
        _ => 3,
    };

    match key.code {
        KeyCode::Tab => {
            state.provider_field_index = (state.provider_field_index + 1) % max_fields;
            WizardTransition::Stay
        }
        KeyCode::BackTab => {
            state.provider_field_index = if state.provider_field_index == 0 {
                max_fields - 1
            } else {
                state.provider_field_index - 1
            };
            WizardTransition::Stay
        }
        KeyCode::Enter => {
            // Validate — only require API key if not detected from env
            state.error_message = None;
            match provider.as_str() {
                "anthropic" | "openai" => {
                    if !has_env_key && state.api_key_input.content.is_empty() {
                        state.error_message = Some("API key is required".to_string());
                        return WizardTransition::Stay;
                    }
                }
                _ => {}
            }
            state.target_field_index = 0; // reset to prompt field
            state.screen = WizardScreen::EnterPrompt;
            WizardTransition::Next(WizardScreen::EnterPrompt)
        }
        _ => {
            // Route to active input
            let input = get_active_input(&provider, has_env_key, state);
            input.handle_key(key);
            WizardTransition::Stay
        }
    }
}

fn get_active_input<'a>(provider: &str, has_env_key: bool, state: &'a mut WizardState) -> &'a mut crate::widgets::input::TextInput {
    match provider {
        "anthropic" => {
            if has_env_key {
                // Fields: model(0), max_turns(1)
                match state.provider_field_index {
                    0 => &mut state.model_input,
                    _ => &mut state.max_turns_input,
                }
            } else {
                // Fields: api_key(0), model(1), max_turns(2)
                match state.provider_field_index {
                    0 => &mut state.api_key_input,
                    1 => &mut state.model_input,
                    _ => &mut state.max_turns_input,
                }
            }
        }
        "openai" => {
            if has_env_key {
                // Fields: model(0), base_url(1), max_turns(2)
                match state.provider_field_index {
                    0 => &mut state.model_input,
                    1 => &mut state.base_url_input,
                    _ => &mut state.max_turns_input,
                }
            } else {
                // Fields: api_key(0), model(1), base_url(2), max_turns(3)
                match state.provider_field_index {
                    0 => &mut state.api_key_input,
                    1 => &mut state.model_input,
                    2 => &mut state.base_url_input,
                    _ => &mut state.max_turns_input,
                }
            }
        }
        "ollama" => match state.provider_field_index {
            0 => &mut state.base_url_input,
            1 => &mut state.model_input,
            _ => &mut state.max_turns_input,
        },
        _ => &mut state.max_turns_input,
    }
}

fn input_snapshot(input: &crate::widgets::input::TextInput, focused: bool) -> InputRender {
    InputRender {
        content: if input.masked {
            "*".repeat(input.content.len())
        } else {
            input.content.clone()
        },
        label: input.label.clone(),
        focused,
        cursor_pos: input.cursor_pos,
    }
}

struct InputRender {
    content: String,
    label: String,
    focused: bool,
    cursor_pos: usize,
}

impl InputRender {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(self.label.as_str())
            .borders(Borders::ALL)
            .border_style(border_style);

        let paragraph = Paragraph::new(self.content.as_str()).block(block);
        paragraph.render(area, buf);

        if self.focused && area.width > 2 && area.height > 0 {
            let cursor_x =
                area.x + 1 + (self.cursor_pos as u16).min(area.width.saturating_sub(3));
            let cursor_y = area.y + 1;
            if let Some(cell) = buf.cell_mut(Position::new(cursor_x, cursor_y)) {
                cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
            }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}
