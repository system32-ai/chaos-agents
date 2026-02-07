use crossterm::event::KeyEvent;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{WizardScreen, WizardState, WizardTransition};
use crate::theme;
use crate::widgets::selector::SelectorAction;

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(" Step 1/6: Select LLM Provider")
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(" Choose the AI provider for chaos planning")
        .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    // We need mutable access to the selector for rendering with StatefulWidget
    // This is a workaround since we get immutable state
    let selector = selector_snapshot(&state.provider_selector);
    selector.render(chunks[2], frame.buffer_mut());

    let help = Paragraph::new(" [Up/Down] Navigate  [Enter] Select  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[3]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    match state.provider_selector.handle_key(key) {
        SelectorAction::Selected(i) => {
            let provider = match i {
                0 => "anthropic",
                1 => "openai",
                2 => "ollama",
                _ => "anthropic",
            };
            state.selected_provider = Some(provider.to_string());

            // Set up provider-specific defaults
            match provider {
                "anthropic" => {
                    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
                    state.api_key_input.set_content(&key);
                    state.model_input.set_content("claude-sonnet-4-5-20250929");
                    state.base_url_input.set_content("");
                }
                "openai" => {
                    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                    state.api_key_input.set_content(&key);
                    state.model_input.set_content("gpt-4o");
                    state.base_url_input.set_content("");
                }
                "ollama" => {
                    state.api_key_input.set_content("");
                    state.model_input.set_content("llama3.1");
                    state.base_url_input.set_content("http://localhost:11434");
                }
                _ => {}
            }
            state.provider_field_index = 0;
            state.screen = WizardScreen::ConfigureProvider;
            WizardTransition::Next(WizardScreen::ConfigureProvider)
        }
        SelectorAction::None => WizardTransition::Stay,
    }
}

// Helper to render selector from immutable state
fn selector_snapshot(selector: &crate::widgets::selector::Selector) -> SelectorRender<'_> {
    SelectorRender {
        items: &selector.items,
        selected: selector.selected_index(),
        label: &selector.label,
    }
}

struct SelectorRender<'a> {
    items: &'a [crate::widgets::selector::SelectorItem],
    selected: usize,
    label: &'a str,
}

impl<'a> SelectorRender<'a> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{List, ListItem};

        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let selected = i == self.selected;
                let prefix = if selected { " > " } else { "   " };
                let hint = item
                    .hint
                    .as_ref()
                    .map(|h| format!(" ({h})"))
                    .unwrap_or_default();
                let line = Line::from(vec![
                    Span::styled(
                        format!("{prefix}{}", item.label),
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
                        format!("      {}", item.description),
                        theme::dim_style(),
                    )),
                ])
            })
            .collect();

        let block = Block::default()
            .title(self.label)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let list = List::new(items).block(block);
        ratatui::widgets::Widget::render(list, area, buf);
    }
}
