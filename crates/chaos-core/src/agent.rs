use async_trait::async_trait;

use crate::discovery::DiscoveredResource;
use crate::error::ChaosResult;
use crate::skill::{Skill, SkillContext, TargetDomain};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Initializing,
    Discovering,
    Ready,
    Executing,
    RollingBack,
    Idle,
    Failed(String),
}

/// An agent manages a collection of skills targeting a specific domain.
#[async_trait]
pub trait Agent: Send + Sync {
    fn domain(&self) -> TargetDomain;

    fn name(&self) -> &str;

    fn status(&self) -> AgentStatus;

    /// Initialize: connect to the target, verify access.
    async fn initialize(&mut self) -> ChaosResult<()>;

    /// Discover resources on the target.
    async fn discover(&mut self) -> ChaosResult<Vec<Box<dyn DiscoveredResource>>>;

    /// Return all skills this agent can perform.
    fn skills(&self) -> Vec<&dyn Skill>;

    /// Look up a skill by name.
    fn skill_by_name(&self, name: &str) -> Option<&dyn Skill>;

    /// Build a SkillContext for executing skills.
    async fn build_context(&self) -> ChaosResult<SkillContext>;

    /// Graceful shutdown: close connections, clean up.
    async fn shutdown(&mut self) -> ChaosResult<()>;
}
