pub mod app;
pub mod dashboard;
pub mod event;
pub mod execution;
pub mod theme;
pub mod widgets;
pub mod wizard;

use std::io;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::Terminal;

use app::{App, AppScreen};
use dashboard::{DashboardAction, DashboardState};
use event::{EventHandler, TuiEvent};
use wizard::WizardTransition;

/// Launch the TUI. This is the entry point called from the CLI.
pub async fn launch_tui() -> anyhow::Result<()> {
    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut app = App::new();
    let mut events = EventHandler::new(std::time::Duration::from_millis(100));

    let mut planner_rx: Option<tokio::sync::mpsc::UnboundedReceiver<_>> = None;
    let mut experiment_rx: Option<tokio::sync::mpsc::UnboundedReceiver<_>> = None;
    let mut task_handle: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        // Render
        terminal.draw(|frame| {
            let area = frame.area();
            match &app.screen {
                AppScreen::Wizard(state) => wizard::render(state, frame, area),
                AppScreen::Dashboard(state) => dashboard::render(state, frame, area),
            }
        })?;

        if app.should_quit {
            break;
        }

        // Handle events based on current screen
        match &app.screen {
            AppScreen::Wizard(_) => {
                if let Some(event) = events.next().await {
                    match event {
                        TuiEvent::Key(key) => {
                            if let AppScreen::Wizard(ref mut state) = app.screen {
                                let transition = wizard::handle_key(state, key);
                                match transition {
                                    WizardTransition::Quit => {
                                        app.should_quit = true;
                                    }
                                    WizardTransition::StartExecution => {
                                        match state.into_output() {
                                            Ok(output) => {
                                                let (p_rx, e_rx, handle) =
                                                    execution::spawn_execution(output.clone());
                                                planner_rx = Some(p_rx);
                                                experiment_rx = Some(e_rx);
                                                task_handle = Some(handle);
                                                app.screen = AppScreen::Dashboard(
                                                    DashboardState::from_wizard_output(output),
                                                );
                                            }
                                            Err(e) => {
                                                state.error_message =
                                                    Some(format!("Error: {e}"));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        TuiEvent::Tick | TuiEvent::Resize(_, _) => {}
                    }
                }
            }
            AppScreen::Dashboard(_) => {
                tokio::select! {
                    event = events.next() => {
                        if let Some(event) = event {
                            match event {
                                TuiEvent::Key(key) => {
                                    if let AppScreen::Dashboard(ref mut state) = app.screen {
                                        let action = dashboard::handle_key(state, key, &mut app.should_quit);
                                        if matches!(action, DashboardAction::CancelExperiment | DashboardAction::CancelAndQuit) {
                                            if let Some(handle) = task_handle.take() {
                                                handle.abort();
                                            }
                                        }
                                    }
                                }
                                TuiEvent::Tick => {
                                    if let AppScreen::Dashboard(ref mut state) = app.screen {
                                        state.tick();
                                    }
                                    // Drain planner events
                                    if let Some(ref mut rx) = planner_rx {
                                        while let Ok(event) = rx.try_recv() {
                                            if let AppScreen::Dashboard(ref mut state) = app.screen {
                                                state.handle_planner_event(event);
                                            }
                                        }
                                    }
                                    // Drain experiment events
                                    if let Some(ref mut rx) = experiment_rx {
                                        while let Ok(event) = rx.try_recv() {
                                            if let AppScreen::Dashboard(ref mut state) = app.screen {
                                                state.handle_experiment_event(event);
                                            }
                                        }
                                    }
                                }
                                TuiEvent::Resize(_, _) => {}
                            }
                        }
                    }
                    Some(event) = async {
                        match planner_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let AppScreen::Dashboard(ref mut state) = app.screen {
                            state.handle_planner_event(event);
                        }
                    }
                    Some(event) = async {
                        match experiment_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let AppScreen::Dashboard(ref mut state) = app.screen {
                            state.handle_experiment_event(event);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
