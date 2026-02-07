use ratatui::style::{Color, Modifier, Style};

pub const BRAND_RED: Color = Color::Rgb(220, 50, 47);
pub const BRAND_ORANGE: Color = Color::Rgb(203, 75, 22);
pub const BRAND_GREEN: Color = Color::Rgb(133, 153, 0);
pub const BRAND_BLUE: Color = Color::Rgb(38, 139, 210);
pub const BRAND_CYAN: Color = Color::Rgb(42, 161, 152);

pub fn title_style() -> Style {
    Style::default().fg(BRAND_RED).add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default().fg(Color::Black).bg(BRAND_BLUE)
}

pub fn normal_style() -> Style {
    Style::default().fg(Color::White)
}

pub fn success_style() -> Style {
    Style::default().fg(BRAND_GREEN)
}

pub fn error_style() -> Style {
    Style::default().fg(BRAND_RED)
}

pub fn dim_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn phase_style(phase: &str) -> Style {
    match phase {
        "Discovering" => Style::default().fg(BRAND_CYAN),
        "Planning" => Style::default().fg(BRAND_BLUE),
        "Executing" => Style::default().fg(BRAND_ORANGE),
        "Waiting" => Style::default().fg(Color::Yellow),
        "RollingBack" => Style::default().fg(BRAND_ORANGE),
        "Complete" => Style::default()
            .fg(BRAND_GREEN)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(BRAND_RED),
    }
}
