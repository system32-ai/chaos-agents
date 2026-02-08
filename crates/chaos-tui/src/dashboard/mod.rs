pub mod status;
pub mod conversation;
pub mod resources;
pub mod progress;
pub mod rollback;
pub mod report;

use std::time::Instant;

use chaos_core::event::ExperimentEvent;
use chaos_llm::planner::PlannerEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme;
use crate::widgets::spinner::Spinner;
use crate::wizard::WizardOutput;

#[derive(Debug, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    CancelExperiment,
    CancelAndQuit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardPhase {
    Planning,
    Discovering,
    Executing,
    Waiting,
    RollingBack,
    Complete,
    Failed(String),
    Cancelled,
}

impl DashboardPhase {
    pub fn label(&self) -> &str {
        match self {
            Self::Planning => "Planning",
            Self::Discovering => "Discovering",
            Self::Executing => "Executing",
            Self::Waiting => "Waiting",
            Self::RollingBack => "RollingBack",
            Self::Complete => "Complete",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Failed(_) | Self::Cancelled
        )
    }
}

pub struct ConversationEntry {
    pub role: String,
    pub content: String,
}

pub struct ResourceEntry {
    pub resource_type: String,
    pub name: String,
}

pub struct SkillProgress {
    pub skill_name: String,
    pub success: Option<bool>,
}

pub struct RollbackProgress {
    pub skill_name: String,
    pub success: Option<bool>,
}

pub struct DashboardState {
    pub phase: DashboardPhase,
    pub wizard_output: WizardOutput,
    pub conversation: Vec<ConversationEntry>,
    pub conversation_scroll: usize,
    pub conversation_auto_scroll: bool,
    /// Cached from last render so key handler knows the max offset.
    pub rendered_max_scroll: std::cell::Cell<usize>,
    pub resources: Vec<ResourceEntry>,
    pub skills: Vec<SkillProgress>,
    pub rollback_steps: Vec<RollbackProgress>,
    pub final_report: Option<String>,
    pub active_panel: usize,
    pub current_turn: u32,
    pub max_turns: u32,
    pub spinner: Spinner,
    pub started_at: Instant,
}

impl DashboardState {
    pub fn from_wizard_output(output: WizardOutput) -> Self {
        Self {
            phase: DashboardPhase::Planning,
            wizard_output: output,
            conversation: Vec::new(),
            conversation_scroll: 0,
            conversation_auto_scroll: true,
            rendered_max_scroll: std::cell::Cell::new(0),
            resources: Vec::new(),
            skills: Vec::new(),
            rollback_steps: Vec::new(),
            final_report: None,
            active_panel: 0,
            current_turn: 0,
            max_turns: 0,
            spinner: Spinner::new(),
            started_at: Instant::now(),
        }
    }

    pub fn handle_planner_event(&mut self, event: PlannerEvent) {
        match event {
            PlannerEvent::TurnStarted { turn, max_turns } => {
                self.current_turn = turn;
                self.max_turns = max_turns;
            }
            PlannerEvent::AssistantMessage { content } => {
                self.conversation.push(ConversationEntry {
                    role: "assistant".into(),
                    content,
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::ToolCallStarted { name, .. } => {
                self.conversation.push(ConversationEntry {
                    role: "tool".into(),
                    content: format!("Calling {}()...", name),
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::ToolCallCompleted {
                name,
                result,
                is_error,
            } => {
                // Parse discover_resources results to populate the resources panel
                if name == "discover_resources" && !is_error {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&result) {
                        if let Some(resources) = json["resources"].as_array() {
                            for r in resources {
                                if let (Some(rtype), Some(rname)) =
                                    (r["type"].as_str(), r["name"].as_str())
                                {
                                    self.resources.push(ResourceEntry {
                                        resource_type: rtype.to_string(),
                                        name: rname.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }

                let prefix = if is_error { "ERROR" } else { "OK" };
                let result_preview = if result.len() > 200 {
                    format!("{}...", &result[..200])
                } else {
                    result
                };
                self.conversation.push(ConversationEntry {
                    role: "tool".into(),
                    content: format!("[{prefix}] {name}: {result_preview}"),
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::DiscoveryResult {
                target,
                resource_count,
            } => {
                self.phase = DashboardPhase::Discovering;
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!(
                        "Discovered {resource_count} resources on {target}"
                    ),
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::ExperimentPlanned { name, target } => {
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Planned: {name} (target: {target})"),
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::PlanningComplete {
                experiment_count, ..
            } => {
                if experiment_count > 0 {
                    self.phase = DashboardPhase::Executing;
                } else {
                    self.phase = DashboardPhase::Complete;
                }
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!(
                        "Planning complete: {experiment_count} experiments"
                    ),
                });
                self.auto_scroll_conversation();
            }
            PlannerEvent::TokenUsage {
                input_tokens,
                output_tokens,
            } => {
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Tokens: {input_tokens} in / {output_tokens} out"),
                });
            }
        }
    }

    pub fn handle_experiment_event(&mut self, event: ExperimentEvent) {
        match event {
            ExperimentEvent::Started { .. } => {
                self.phase = DashboardPhase::Executing;
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: "Experiment started".into(),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::SkillExecuted {
                skill_name,
                success,
                ..
            } => {
                let status = if success { "OK" } else { "FAILED" };
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Skill {skill_name}: {status}"),
                });
                self.skills.push(SkillProgress {
                    skill_name,
                    success: Some(success),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::DurationWaitBegin { duration, .. } => {
                self.phase = DashboardPhase::Waiting;
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Waiting for {duration:?}..."),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::RollbackStarted { .. } => {
                self.phase = DashboardPhase::RollingBack;
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: "Rolling back...".into(),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::RollbackStepCompleted {
                skill_name,
                success,
                ..
            } => {
                let status = if success { "OK" } else { "FAILED" };
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Rollback {skill_name}: {status}"),
                });
                self.rollback_steps.push(RollbackProgress {
                    skill_name,
                    success: Some(success),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::Completed { .. } => {
                self.phase = DashboardPhase::Complete;
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: "Experiment completed successfully!".into(),
                });
                self.auto_scroll_conversation();
            }
            ExperimentEvent::Failed { error, .. } => {
                self.conversation.push(ConversationEntry {
                    role: "system".into(),
                    content: format!("Experiment failed: {error}"),
                });
                self.phase = DashboardPhase::Failed(error);
                self.auto_scroll_conversation();
            }
        }
    }

    fn auto_scroll_conversation(&mut self) {
        self.conversation_auto_scroll = true;
    }

    pub fn elapsed_display(&self) -> String {
        let secs = self.started_at.elapsed().as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
        }
    }

    pub fn tick(&mut self) {
        self.spinner.tick();
    }
}

pub fn render(state: &DashboardState, frame: &mut Frame, area: Rect) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    // Status bar
    status::render(state, frame, main_chunks[0]);

    // Main content: 2x2 grid
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(content_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content_chunks[1]);

    // Conversation (top-left, larger)
    conversation::render(state, frame, left_chunks[0], state.active_panel == 0);

    // Skill progress (bottom-left)
    progress::render(state, frame, left_chunks[1], state.active_panel == 2);

    // Resources (top-right)
    resources::render(state, frame, right_chunks[0], state.active_panel == 1);

    // Rollback (bottom-right)
    rollback::render(state, frame, right_chunks[1], state.active_panel == 3);

    // Help bar
    let help_text = if state.phase.is_finished() {
        " [q] Quit  [Tab] Switch panel  [Up/Down] Scroll"
    } else {
        " [Ctrl+C] Cancel  [Ctrl+W] Cancel & Quit  [Tab] Panel  [Up/Down] Scroll"
    };
    let help = Paragraph::new(help_text).style(theme::dim_style());
    frame.render_widget(help, main_chunks[2]);
}

pub fn handle_key(state: &mut DashboardState, key: KeyEvent, should_quit: &mut bool) -> DashboardAction {
    // Ctrl+C: cancel experiment, stay in TUI
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if !state.phase.is_finished() {
            state.phase = DashboardPhase::Cancelled;
            state.conversation.push(ConversationEntry {
                role: "system".into(),
                content: "Experiment cancelled by user (Ctrl+C)".into(),
            });
            state.auto_scroll_conversation();
            return DashboardAction::CancelExperiment;
        }
        return DashboardAction::None;
    }

    // Ctrl+W: cancel experiment and quit TUI
    if key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if !state.phase.is_finished() {
            state.phase = DashboardPhase::Cancelled;
            state.conversation.push(ConversationEntry {
                role: "system".into(),
                content: "Experiment cancelled, closing TUI (Ctrl+W)".into(),
            });
        }
        *should_quit = true;
        return DashboardAction::CancelAndQuit;
    }

    match key.code {
        KeyCode::Char('q') => {
            if state.phase.is_finished() {
                *should_quit = true;
            }
        }
        KeyCode::Tab => {
            state.active_panel = (state.active_panel + 1) % 4;
        }
        KeyCode::Up => {
            if state.active_panel == 0 {
                if state.conversation_auto_scroll {
                    // Switch from auto-scroll to manual, start near the bottom
                    state.conversation_auto_scroll = false;
                    state.conversation_scroll = state.rendered_max_scroll.get().saturating_sub(1);
                } else {
                    state.conversation_scroll = state.conversation_scroll.saturating_sub(1);
                }
            }
        }
        KeyCode::Down => {
            if state.active_panel == 0 {
                if !state.conversation_auto_scroll {
                    state.conversation_scroll += 1;
                    if state.conversation_scroll >= state.rendered_max_scroll.get() {
                        state.conversation_auto_scroll = true;
                    }
                }
            }
        }
        _ => {}
    }
    DashboardAction::None
}
