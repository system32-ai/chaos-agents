use crate::dashboard::DashboardState;
use crate::wizard::WizardState;

pub enum AppScreen {
    Wizard(WizardState),
    Dashboard(DashboardState),
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            screen: AppScreen::Wizard(WizardState::new()),
            should_quit: false,
        }
    }
}
