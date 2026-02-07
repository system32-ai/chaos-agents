use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use super::DashboardState;
use crate::theme;

pub fn render(state: &DashboardState, frame: &mut Frame, area: Rect, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(format!(" Resources ({}) ", state.resources.len()))
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.resources.is_empty() {
        let empty = Paragraph::new("  No resources discovered yet")
            .style(theme::dim_style())
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = state
        .resources
        .iter()
        .map(|r| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  [{:>10}] ", r.resource_type),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(&r.name, theme::normal_style()),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
