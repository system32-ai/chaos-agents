use async_trait::async_trait;

use chaos_core::agent::{Agent, AgentStatus};
use chaos_core::discovery::DiscoveredResource;
use chaos_core::error::ChaosResult;
use chaos_core::skill::{Skill, SkillContext, TargetDomain};

use crate::config::ServerTargetConfig;
use crate::service_discovery::ServiceDiscoverer;
use crate::skills::cpu_stress::CpuStressSkill;
use crate::skills::disk_fill::DiskFillSkill;
use crate::skills::memory_stress::MemoryStressSkill;
use crate::skills::permission_change::PermissionChangeSkill;
use crate::skills::service_stop::ServiceStopSkill;
use crate::ssh::SshSession;

pub struct ServerAgent {
    config: ServerTargetConfig,
    sessions: Vec<SshSession>,
    status: AgentStatus,
    skills: Vec<Box<dyn Skill>>,
}

impl ServerAgent {
    pub fn new(config: ServerTargetConfig) -> Self {
        let skills: Vec<Box<dyn Skill>> = vec![
            Box::new(DiskFillSkill),
            Box::new(PermissionChangeSkill),
            Box::new(ServiceStopSkill),
            Box::new(CpuStressSkill),
            Box::new(MemoryStressSkill),
        ];
        Self {
            config,
            sessions: Vec::new(),
            status: AgentStatus::Idle,
            skills,
        }
    }

    pub fn from_yaml(value: &serde_yaml::Value) -> ChaosResult<Self> {
        let config: ServerTargetConfig = serde_yaml::from_value(value.clone())
            .map_err(|e| chaos_core::error::ChaosError::Config(format!("Invalid server config: {e}")))?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl Agent for ServerAgent {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Server
    }

    fn name(&self) -> &str {
        "server-chaos-agent"
    }

    fn status(&self) -> AgentStatus {
        self.status.clone()
    }

    async fn initialize(&mut self) -> ChaosResult<()> {
        self.status = AgentStatus::Initializing;

        for host_config in &self.config.hosts {
            let session = SshSession::connect(host_config)
                .await
                .map_err(|e| {
                    chaos_core::error::ChaosError::Connection(anyhow::anyhow!(
                        "SSH connection to {} failed: {e}",
                        host_config.host
                    ))
                })?;
            tracing::info!(host = %host_config.host, "SSH connection established");
            self.sessions.push(session);
        }

        self.status = AgentStatus::Ready;
        tracing::info!(hosts = self.sessions.len(), "Server agent initialized");
        Ok(())
    }

    async fn discover(&mut self) -> ChaosResult<Vec<Box<dyn DiscoveredResource>>> {
        self.status = AgentStatus::Discovering;

        if !self.config.discovery.enabled {
            self.status = AgentStatus::Ready;
            return Ok(Vec::new());
        }

        let mut all_resources: Vec<Box<dyn DiscoveredResource>> = Vec::new();

        for session in &self.sessions {
            let resources = ServiceDiscoverer::discover(
                session,
                &self.config.discovery.exclude_services,
            )
            .await
            .map_err(|e| {
                chaos_core::error::ChaosError::Discovery(format!(
                    "Discovery on {} failed: {e}",
                    session.host
                ))
            })?;

            tracing::info!(
                host = %session.host,
                services = resources.iter().filter(|r| r.resource_type() == "service").count(),
                ports = resources.iter().filter(|r| r.resource_type() == "port").count(),
                filesystems = resources.iter().filter(|r| r.resource_type() == "filesystem").count(),
                "Server discovery complete"
            );

            for r in resources {
                all_resources.push(Box::new(r));
            }
        }

        self.status = AgentStatus::Ready;
        Ok(all_resources)
    }

    fn skills(&self) -> Vec<&dyn Skill> {
        self.skills.iter().map(|s| s.as_ref()).collect()
    }

    fn skill_by_name(&self, name: &str) -> Option<&dyn Skill> {
        self.skills
            .iter()
            .find(|s| s.descriptor().name == name)
            .map(|s| s.as_ref())
    }

    async fn build_context(&self) -> ChaosResult<SkillContext> {
        // Use first session for now. A more advanced implementation would
        // select based on the target host from the skill invocation.
        let session = self
            .sessions
            .first()
            .ok_or_else(|| {
                chaos_core::error::ChaosError::Connection(anyhow::anyhow!("No SSH sessions"))
            })?;

        // We can't move the session, so we create a new connection for the context.
        // In a production implementation, we'd use an Arc<SshSession> pool.
        let host_config = self.config.hosts.first().ok_or_else(|| {
            chaos_core::error::ChaosError::Connection(anyhow::anyhow!("No host configs"))
        })?;

        let new_session = SshSession::connect(host_config)
            .await
            .map_err(|e| {
                chaos_core::error::ChaosError::Connection(anyhow::anyhow!(
                    "SSH reconnect to {} failed: {e}",
                    session.host
                ))
            })?;

        Ok(SkillContext {
            shared: Box::new(new_session),
            params: serde_yaml::Value::Null,
        })
    }

    async fn shutdown(&mut self) -> ChaosResult<()> {
        self.sessions.clear();
        self.status = AgentStatus::Idle;
        tracing::info!("Server agent shut down");
        Ok(())
    }
}
