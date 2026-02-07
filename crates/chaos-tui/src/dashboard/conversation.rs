use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::DashboardState;
use crate::theme;

pub fn render(state: &DashboardState, frame: &mut Frame, area: Rect, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Chat ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.conversation.is_empty() {
        let empty = Paragraph::new("  Waiting for LLM response...")
            .style(theme::dim_style())
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let lines: Vec<Line> = state
        .conversation
        .iter()
        .map(|entry| {
            let (prefix, style) = match entry.role.as_str() {
                "assistant" => ("AI", Style::default().fg(Color::Green)),
                "tool" => (">>", Style::default().fg(Color::Yellow)),
                "system" => ("**", Style::default().fg(Color::Cyan)),
                _ => ("  ", theme::normal_style()),
            };

            Line::from(vec![
                Span::styled(format!("[{prefix}] "), style),
                Span::styled(entry.content.clone(), theme::normal_style()),
            ])
        })
        .collect();

    let scroll_y = state.conversation_scroll.min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));
    frame.render_widget(paragraph, area);
}
