use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::DashboardState;
use crate::theme;

pub fn render(state: &DashboardState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Experiment Report ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let content = state
        .final_report
        .as_deref()
        .unwrap_or("Report will appear here when experiment completes.");

    let paragraph = Paragraph::new(content)
        .style(theme::normal_style())
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}
