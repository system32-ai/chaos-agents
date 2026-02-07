use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::DashboardState;
use crate::theme;

pub fn render(state: &DashboardState, frame: &mut Frame, area: Rect) {
    let phase_label = state.phase.label();
    let spinner = if phase_label != "Complete" && !phase_label.starts_with("Failed") {
        format!("{} ", state.spinner.frame())
    } else {
        String::new()
    };

    let turn_info = if state.max_turns > 0 {
        format!("Turn {}/{}", state.current_turn, state.max_turns)
    } else {
        String::new()
    };

    let phase_style = theme::phase_style(phase_label);

    let status_line = Line::from(vec![
        Span::styled(format!(" {spinner}Phase: "), Style::default().fg(Color::White)),
        Span::styled(format!("[{phase_label}]"), phase_style),
        Span::raw("  "),
        Span::styled(
            format!("Duration: {}", state.wizard_output.duration),
            theme::dim_style(),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Skills: {} executed", state.skills.len()),
            theme::dim_style(),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Rollbacks: {}", state.rollback_steps.len()),
            theme::dim_style(),
        ),
        Span::raw("  "),
        Span::styled(turn_info, theme::dim_style()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Chaos Agents Dashboard ");

    let paragraph = Paragraph::new(status_line).block(block);
    frame.render_widget(paragraph, area);
}
