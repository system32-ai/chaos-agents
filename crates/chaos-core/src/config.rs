use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{ChaosError, ChaosResult};
use crate::experiment::ExperimentConfig;

/// Top-level config file structure.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChaosConfig {
    pub experiments: Vec<ExperimentConfig>,
}

/// Daemon-mode schedule config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub experiments: Vec<ScheduledExperiment>,
    #[serde(default)]
    pub settings: DaemonSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledExperiment {
    pub experiment: ExperimentConfig,
    /// Cron expression, e.g. "0 */30 * * * *"
    pub schedule: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSettings {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    pub health_bind: Option<String>,
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            health_bind: None,
        }
    }
}

fn default_max_concurrent() -> usize {
    2
}

impl ChaosConfig {
    pub fn from_file(path: &Path) -> ChaosResult<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ChaosError::Config(format!("Cannot read {}: {e}", path.display())))?;
        serde_yaml::from_str(&content)
            .map_err(|e| ChaosError::Config(format!("Invalid YAML: {e}")))
    }
}

impl DaemonConfig {
    pub fn from_file(path: &Path) -> ChaosResult<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ChaosError::Config(format!("Cannot read {}: {e}", path.display())))?;
        serde_yaml::from_str(&content)
            .map_err(|e| ChaosError::Config(format!("Invalid YAML: {e}")))
    }
}
