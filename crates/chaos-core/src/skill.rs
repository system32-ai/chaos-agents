use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt;

use crate::error::ChaosResult;
use crate::rollback::RollbackHandle;

/// Metadata describing a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub target: TargetDomain,
    pub reversible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetDomain {
    Database,
    Kubernetes,
    Server,
}

impl fmt::Display for TargetDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database => write!(f, "database"),
            Self::Kubernetes => write!(f, "kubernetes"),
            Self::Server => write!(f, "server"),
        }
    }
}

/// Context passed into skill execution.
pub struct SkillContext {
    /// Agent-specific shared state (downcast by the skill).
    pub shared: Box<dyn Any + Send + Sync>,
    /// Parameters from the YAML config for this skill invocation.
    pub params: serde_yaml::Value,
}

/// A single reversible chaos action.
#[async_trait]
pub trait Skill: Send + Sync {
    fn descriptor(&self) -> SkillDescriptor;

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()>;

    /// Execute the chaos action. Returns a handle for rollback.
    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle>;

    /// Reverse a previously executed action.
    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()>;
}
