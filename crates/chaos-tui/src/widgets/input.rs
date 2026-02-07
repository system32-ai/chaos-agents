use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub struct TextInput {
    pub content: String,
    pub cursor_pos: usize,
    pub multiline: bool,
    pub label: String,
    pub focused: bool,
    pub masked: bool,
}

impl TextInput {
    pub fn new(label: &str) -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            multiline: false,
            label: label.to_string(),
            focused: false,
            masked: false,
        }
    }

    pub fn with_multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    pub fn with_masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn with_content(mut self, content: &str) -> Self {
        self.content = content.to_string();
        self.cursor_pos = content.len();
        self
    }

    pub fn set_content(&mut self, content: &str) {
        self.content = content.to_string();
        self.cursor_pos = content.len();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                InputAction::Submit
            }
            KeyCode::Enter if !self.multiline => InputAction::Submit,
            KeyCode::Enter if self.multiline => {
                self.content.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
                InputAction::Changed
            }
            KeyCode::Char(c) => {
                self.content.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                InputAction::Changed
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.content.remove(self.cursor_pos);
                }
                InputAction::Changed
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.content.len() {
                    self.content.remove(self.cursor_pos);
                }
                InputAction::Changed
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                InputAction::None
            }
            KeyCode::Right => {
                if self.cursor_pos < self.content.len() {
                    self.cursor_pos += 1;
                }
                InputAction::None
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                InputAction::None
            }
            KeyCode::End => {
                self.cursor_pos = self.content.len();
                InputAction::None
            }
            _ => InputAction::None,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let display_text = if self.masked {
            "*".repeat(self.content.len())
        } else {
            self.content.clone()
        };

        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(self.label.as_str())
            .borders(Borders::ALL)
            .border_style(border_style);

        let paragraph = Paragraph::new(display_text).block(block);
        paragraph.render(area, buf);

        // Show cursor position when focused
        if self.focused && area.width > 2 && area.height > 2 {
            let cursor_x = area.x + 1 + (self.cursor_pos as u16).min(area.width.saturating_sub(3));
            let cursor_y = area.y + 1;
            if let Some(cell) = buf.cell_mut(Position::new(cursor_x, cursor_y)) {
                cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
            }
        }
    }
}

pub enum InputAction {
    None,
    Changed,
    Submit,
}
