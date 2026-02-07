use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::theme;

pub struct SelectorItem {
    pub label: String,
    pub description: String,
    pub hint: Option<String>,
}

pub struct Selector {
    pub items: Vec<SelectorItem>,
    pub state: ListState,
    pub label: String,
}

impl Selector {
    pub fn new(label: &str, items: Vec<SelectorItem>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items,
            state,
            label: label.to_string(),
        }
    }

    pub fn selected_index(&self) -> usize {
        self.state.selected().unwrap_or(0)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SelectorAction {
        let len = self.items.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.selected_index();
                let next = if i == 0 { len - 1 } else { i - 1 };
                self.state.select(Some(next));
                SelectorAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.selected_index();
                let next = (i + 1) % len;
                self.state.select(Some(next));
                SelectorAction::None
            }
            KeyCode::Enter => SelectorAction::Selected(self.selected_index()),
            _ => SelectorAction::None,
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let selected = self.state.selected() == Some(i);
                let prefix = if selected { ">" } else { " " };
                let hint = item
                    .hint
                    .as_ref()
                    .map(|h| format!(" ({h})"))
                    .unwrap_or_default();
                let line = Line::from(vec![
                    Span::styled(
                        format!("{prefix} {}", item.label),
                        if selected {
                            theme::selected_style()
                        } else {
                            theme::normal_style()
                        },
                    ),
                    Span::styled(hint, theme::dim_style()),
                ]);
                ListItem::new(vec![
                    line,
                    Line::from(Span::styled(
                        format!("    {}", item.description),
                        theme::dim_style(),
                    )),
                ])
            })
            .collect();

        let block = Block::default()
            .title(self.label.as_str())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let list = List::new(items).block(block);
        ratatui::widgets::StatefulWidget::render(list, area, buf, &mut self.state);
    }
}

pub enum SelectorAction {
    None,
    Selected(usize),
}
