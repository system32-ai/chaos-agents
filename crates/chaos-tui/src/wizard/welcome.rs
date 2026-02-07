use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{WizardState, WizardScreen, WizardTransition};
use crate::theme;

const BANNER: &str = r#"
   _____ _    _          ____   _____
  / ____| |  | |   /\   / __ \ / ____|
 | |    | |__| |  /  \ | |  | | (___
 | |    |  __  | / /\ \| |  | |\___ \
 | |____| |  | |/ ____ \ |__| |____) |
  \_____|_|  |_/_/    \_\____/|_____/
     /\   _____ ______ _   _ _______ _____
    /  \ / ____|  ____| \ | |__   __/ ____|
   / /\ \ |  __| |__  |  \| |  | | | (___
  / ____ \ | |_ |  __| | . ` |  | |  \___ \
 / /    \ \ |__| | |____| |\  |  | |  ____) |
/_/      \_\_____|______|_| \_|  |_| |_____/
"#;

pub fn render(_state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(14),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);

    // Title
    let title = Paragraph::new("").block(
        Block::default()
            .borders(Borders::NONE),
    );
    frame.render_widget(title, chunks[0]);

    // Banner
    let banner = Paragraph::new(BANNER)
        .style(theme::title_style())
        .alignment(Alignment::Center);
    frame.render_widget(banner, chunks[1]);

    // Description
    let desc = Paragraph::new(
        "Controlled chaos engineering for databases, Kubernetes, and servers",
    )
    .style(theme::dim_style())
    .alignment(Alignment::Center);
    frame.render_widget(desc, chunks[2]);

    // Help
    let help = Paragraph::new("Press Enter to start  |  q to quit")
        .style(theme::dim_style())
        .alignment(Alignment::Center);
    frame.render_widget(help, chunks[4]);
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    match key.code {
        KeyCode::Enter => {
            state.screen = WizardScreen::SelectProvider;
            WizardTransition::Next(WizardScreen::SelectProvider)
        }
        KeyCode::Char('q') => WizardTransition::Quit,
        _ => WizardTransition::Stay,
    }
}
