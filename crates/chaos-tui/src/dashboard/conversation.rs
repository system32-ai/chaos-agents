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

    // Estimate total wrapped display rows so we can compute max scroll.
    // inner_width excludes the 2 border columns.
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;

    let total_rows: usize = lines
        .iter()
        .map(|line| {
            let len: usize = line.spans.iter().map(|s| s.content.len()).sum();
            if inner_width == 0 {
                1
            } else {
                (len.max(1) + inner_width - 1) / inner_width
            }
        })
        .sum();

    let max_scroll = total_rows.saturating_sub(inner_height);
    state.rendered_max_scroll.set(max_scroll);

    let scroll_y = if state.conversation_auto_scroll {
        max_scroll
    } else {
        state.conversation_scroll.min(max_scroll)
    } as u16;

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));
    frame.render_widget(paragraph, area);
}
