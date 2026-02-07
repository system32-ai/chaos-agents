use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{WizardScreen, WizardState, WizardTransition};
use crate::theme;

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let target = state.selected_target.as_deref().unwrap_or("unknown");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(format!(
        " Step 4/6: Configure {} Target",
        capitalize(target)
    ))
    .style(theme::title_style())
    .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(" Enter connection details for the target")
        .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    match target {
        "database" => {
            render_input(
                &state.db_url_input,
                state.target_field_index == 0,
                chunks[2],
                frame.buffer_mut(),
            );
            // Show db type as text indicator
            let db_type = match state.db_type_selector.selected_index() {
                0 => "postgres",
                1 => "mysql",
                2 => "mongodb",
                _ => "postgres",
            };
            let db_block = Block::default()
                .title(" DB Type (j/k to change) ")
                .borders(Borders::ALL)
                .border_style(if state.target_field_index == 1 {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                });
            let db_para = Paragraph::new(format!("  {db_type}")).block(db_block);
            frame.render_widget(db_para, chunks[3]);

            render_input(
                &state.db_schemas_input,
                state.target_field_index == 2,
                chunks[4],
                frame.buffer_mut(),
            );
        }
        "kubernetes" => {
            render_input(
                &state.k8s_namespace_input,
                state.target_field_index == 0,
                chunks[2],
                frame.buffer_mut(),
            );
            render_input(
                &state.k8s_label_input,
                state.target_field_index == 1,
                chunks[3],
                frame.buffer_mut(),
            );
            render_input(
                &state.k8s_kubeconfig_input,
                state.target_field_index == 2,
                chunks[4],
                frame.buffer_mut(),
            );
        }
        "server" => {
            render_input(
                &state.server_host_input,
                state.target_field_index == 0,
                chunks[2],
                frame.buffer_mut(),
            );
            render_input(
                &state.server_port_input,
                state.target_field_index == 1,
                chunks[3],
                frame.buffer_mut(),
            );
            render_input(
                &state.server_username_input,
                state.target_field_index == 2,
                chunks[4],
                frame.buffer_mut(),
            );
            // Auth type
            let auth_type = if state.server_auth_selector.selected_index() == 0 {
                "SSH Key"
            } else {
                "Password"
            };
            let auth_block = Block::default()
                .title(" Auth Type (j/k to change) ")
                .borders(Borders::ALL)
                .border_style(if state.target_field_index == 3 {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                });
            let auth_para = Paragraph::new(format!("  {auth_type}")).block(auth_block);
            frame.render_widget(auth_para, chunks[5]);

            let auth_label = if state.server_auth_selector.selected_index() == 0 {
                " Key Path "
            } else {
                " Password "
            };
            // Update label dynamically
            let auth_input = input_render_with_label(
                &state.server_auth_value_input,
                state.target_field_index == 4,
                auth_label,
            );
            auth_input.render(chunks[6], frame.buffer_mut());
        }
        _ => {}
    }

    // Error
    if let Some(ref err) = state.error_message {
        let error = Paragraph::new(format!(" Error: {err}")).style(theme::error_style());
        frame.render_widget(error, chunks[7]);
    }

    let help = Paragraph::new(" [Tab] Next field  [Enter] Continue  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[8]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    let target = state
        .selected_target
        .as_deref()
        .unwrap_or("unknown")
        .to_string();

    let max_fields = match target.as_str() {
        "database" => 3,
        "kubernetes" => 3,
        "server" => 5,
        _ => 1,
    };

    match key.code {
        KeyCode::Tab => {
            state.target_field_index = (state.target_field_index + 1) % max_fields;
            WizardTransition::Stay
        }
        KeyCode::BackTab => {
            state.target_field_index = if state.target_field_index == 0 {
                max_fields - 1
            } else {
                state.target_field_index - 1
            };
            WizardTransition::Stay
        }
        KeyCode::Enter => {
            state.error_message = None;
            // Validate required fields
            match target.as_str() {
                "database" => {
                    if state.db_url_input.content.is_empty() {
                        state.error_message = Some("Connection URL is required".to_string());
                        return WizardTransition::Stay;
                    }
                }
                "server" => {
                    if state.server_host_input.content.is_empty() {
                        state.error_message = Some("Host is required".to_string());
                        return WizardTransition::Stay;
                    }
                    if state.server_username_input.content.is_empty() {
                        state.error_message = Some("Username is required".to_string());
                        return WizardTransition::Stay;
                    }
                }
                _ => {}
            }
            state.screen = WizardScreen::EnterPrompt;
            WizardTransition::Next(WizardScreen::EnterPrompt)
        }
        _ => {
            // Handle selector fields (db_type, auth_type) with j/k
            match target.as_str() {
                "database" if state.target_field_index == 1 => {
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            state.db_type_selector.handle_key(key);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            state.db_type_selector.handle_key(key);
                        }
                        _ => {}
                    }
                    return WizardTransition::Stay;
                }
                "server" if state.target_field_index == 3 => {
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            state.server_auth_selector.handle_key(key);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            state.server_auth_selector.handle_key(key);
                        }
                        _ => {}
                    }
                    return WizardTransition::Stay;
                }
                _ => {}
            }

            // Route to active text input
            if let Some(input) = get_active_target_input(&target, state) {
                input.handle_key(key);
            }
            WizardTransition::Stay
        }
    }
}

fn get_active_target_input<'a>(
    target: &str,
    state: &'a mut WizardState,
) -> Option<&'a mut crate::widgets::input::TextInput> {
    match target {
        "database" => match state.target_field_index {
            0 => Some(&mut state.db_url_input),
            2 => Some(&mut state.db_schemas_input),
            _ => None, // index 1 is selector
        },
        "kubernetes" => match state.target_field_index {
            0 => Some(&mut state.k8s_namespace_input),
            1 => Some(&mut state.k8s_label_input),
            2 => Some(&mut state.k8s_kubeconfig_input),
            _ => None,
        },
        "server" => match state.target_field_index {
            0 => Some(&mut state.server_host_input),
            1 => Some(&mut state.server_port_input),
            2 => Some(&mut state.server_username_input),
            4 => Some(&mut state.server_auth_value_input),
            _ => None, // index 3 is selector
        },
        _ => None,
    }
}

fn render_input(
    input: &crate::widgets::input::TextInput,
    focused: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let display = if input.masked {
        "*".repeat(input.content.len())
    } else {
        input.content.clone()
    };

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(input.label.as_str())
        .borders(Borders::ALL)
        .border_style(border_style);

    let paragraph = Paragraph::new(display).block(block);
    paragraph.render(area, buf);

    if focused && area.width > 2 {
        let cursor_x = area.x + 1 + (input.cursor_pos as u16).min(area.width.saturating_sub(3));
        let cursor_y = area.y + 1;
        if let Some(cell) = buf.cell_mut(Position::new(cursor_x, cursor_y)) {
            cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
        }
    }
}

fn input_render_with_label<'a>(
    input: &'a crate::widgets::input::TextInput,
    focused: bool,
    label: &'a str,
) -> InputRenderCustom<'a> {
    InputRenderCustom {
        content: if input.masked {
            "*".repeat(input.content.len())
        } else {
            input.content.clone()
        },
        label,
        focused,
        cursor_pos: input.cursor_pos,
    }
}

struct InputRenderCustom<'a> {
    content: String,
    label: &'a str,
    focused: bool,
    cursor_pos: usize,
}

impl<'a> InputRenderCustom<'a> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(self.label)
            .borders(Borders::ALL)
            .border_style(border_style);

        let paragraph = Paragraph::new(self.content.as_str()).block(block);
        paragraph.render(area, buf);

        if self.focused && area.width > 2 {
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
