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
        .title(" LLM Conversation ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.conversation.is_empty() {
        let empty = Paragraph::new("  Waiting for LLM response...")
            .style(theme::dim_style())
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let start = if state.conversation.len() > inner_height {
        state
            .conversation_scroll
            .min(state.conversation.len().saturating_sub(inner_height))
    } else {
        0
    };

    let items: Vec<ListItem> = state
        .conversation
        .iter()
        .skip(start)
        .take(inner_height)
        .map(|entry| {
            let (prefix, style) = match entry.role.as_str() {
                "assistant" => ("AI", Style::default().fg(Color::Green)),
                "tool" => (">>", Style::default().fg(Color::Yellow)),
                "system" => ("**", Style::default().fg(Color::Cyan)),
                _ => ("  ", theme::normal_style()),
            };

            // Truncate long lines for display
            let content = if entry.content.len() > 120 {
                format!("{}...", &entry.content[..120])
            } else {
                entry.content.clone()
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("[{prefix}] "), style),
                Span::styled(content, theme::normal_style()),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
