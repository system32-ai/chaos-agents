use crossterm::event::KeyEvent;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

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

    let title = Paragraph::new(" Step 3/6: Select Target")
        .style(theme::title_style())
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(title, chunks[0]);

    let subtitle = Paragraph::new(" Choose the infrastructure to test")
        .style(theme::dim_style());
    frame.render_widget(subtitle, chunks[1]);

    // Render selector snapshot
    let items: Vec<ListItem> = state
        .target_selector
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = state.target_selector.selected_index() == i;
            let prefix = if selected { " > " } else { "   " };
            let line = Line::from(vec![Span::styled(
                format!("{prefix}{}", item.label),
                if selected {
                    theme::selected_style()
                } else {
                    theme::normal_style()
                },
            )]);
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
        .title(" Select Target ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);
    frame.render_widget(list, chunks[2]);

    let help = Paragraph::new(" [Up/Down] Navigate  [Enter] Select  [Esc] Back")
        .style(theme::dim_style());
    frame.render_widget(help, chunks[3]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    match state.target_selector.handle_key(key) {
        SelectorAction::Selected(i) => {
            let target = match i {
                0 => "database",
                1 => "kubernetes",
                2 => "server",
                _ => "database",
            };
            state.selected_target = Some(target.to_string());
            state.target_field_index = 0;
            state.screen = WizardScreen::ConfigureTarget;
            WizardTransition::Next(WizardScreen::ConfigureTarget)
        }
        SelectorAction::None => WizardTransition::Stay,
    }
}
