use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::rollback::RollbackLog;
use crate::skill::TargetDomain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentConfig {
    pub name: String,
    pub target: TargetDomain,
    /// Target-specific connection/auth config (parsed by the agent).
    pub target_config: serde_yaml::Value,
    /// Which skills to run and their parameters.
    pub skills: Vec<SkillInvocation>,
    /// How long to let the chaos run before triggering rollback.
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    /// Whether to run skills in parallel or sequentially.
    #[serde(default)]
    pub parallel: bool,
    /// Only target discovered resources matching these regex patterns.
    #[serde(default)]
    pub resource_filters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInvocation {
    pub skill_name: String,
    #[serde(default)]
    pub params: serde_yaml::Value,
    #[serde(default = "default_count")]
    pub count: u32,
}

fn default_count() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentStatus {
    Pending,
    Discovering,
    Executing,
    WaitingDuration,
    RollingBack,
    Completed,
    Failed(String),
}

/// Runtime state of a running experiment.
pub struct Experiment {
    pub id: Uuid,
    pub config: ExperimentConfig,
    pub status: ExperimentStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub rollback_log: RollbackLog,
}

impl Experiment {
    pub fn new(config: ExperimentConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            config,
            status: ExperimentStatus::Pending,
            started_at: None,
            completed_at: None,
            rollback_log: RollbackLog::new(),
        }
    }
}
