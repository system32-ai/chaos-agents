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
        .title(format!(" Rollback ({}) ", state.rollback_steps.len()))
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.rollback_steps.is_empty() {
        let label = if state.phase == super::DashboardPhase::RollingBack {
            "  Rolling back..."
        } else {
            "  No rollbacks yet"
        };
        let empty = Paragraph::new(label)
            .style(theme::dim_style())
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = state
        .rollback_steps
        .iter()
        .map(|r| {
            let (icon, style) = match r.success {
                Some(true) => ("OK", theme::success_style()),
                Some(false) => ("FAIL", theme::error_style()),
                None => ("...", Style::default().fg(Color::Yellow)),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  [{icon:>4}] "), style),
                Span::styled(&r.skill_name, theme::normal_style()),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
