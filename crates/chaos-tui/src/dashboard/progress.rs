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
        .title(format!(" Skill Execution ({}) ", state.skills.len()))
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.skills.is_empty() {
        let empty = Paragraph::new("  No skills executed yet")
            .style(theme::dim_style())
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = state
        .skills
        .iter()
        .map(|s| {
            let (icon, style) = match s.success {
                Some(true) => ("OK", theme::success_style()),
                Some(false) => ("FAIL", theme::error_style()),
                None => ("...", Style::default().fg(Color::Yellow)),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  [{icon:>4}] "), style),
                Span::styled(&s.skill_name, theme::normal_style()),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
