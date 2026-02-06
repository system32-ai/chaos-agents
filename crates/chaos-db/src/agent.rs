use async_trait::async_trait;
use sqlx::any::AnyPool;

use chaos_core::agent::{Agent, AgentStatus};
use chaos_core::discovery::DiscoveredResource;
use chaos_core::error::ChaosResult;
use chaos_core::skill::{Skill, SkillContext, TargetDomain};

use crate::config::{DbTargetConfig, DbType};
use crate::connection::create_pool;
use crate::schema_discovery::discover_schema;
use crate::skills::config_change::ConfigChangeSkill;
use crate::skills::insert_load::InsertLoadSkill;
use crate::skills::select_load::SelectLoadSkill;
use crate::skills::update_load::UpdateLoadSkill;

pub struct DbAgent {
    config: DbTargetConfig,
    pool: Option<AnyPool>,
    status: AgentStatus,
    skills: Vec<Box<dyn Skill>>,
}

impl DbAgent {
    pub fn new(config: DbTargetConfig) -> Self {
        let db_type = config.db_type;
        let skills: Vec<Box<dyn Skill>> = vec![
            Box::new(InsertLoadSkill),
            Box::new(UpdateLoadSkill),
            Box::new(SelectLoadSkill),
            Box::new(ConfigChangeSkill { db_type }),
        ];
        Self {
            config,
            pool: None,
            status: AgentStatus::Idle,
            skills,
        }
    }

    pub fn from_yaml(value: &serde_yaml::Value) -> ChaosResult<Self> {
        let config: DbTargetConfig = serde_yaml::from_value(value.clone())
            .map_err(|e| chaos_core::error::ChaosError::Config(format!("Invalid DB config: {e}")))?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl Agent for DbAgent {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Database
    }

    fn name(&self) -> &str {
        "database-chaos-agent"
    }

    fn status(&self) -> AgentStatus {
        self.status.clone()
    }

    async fn initialize(&mut self) -> ChaosResult<()> {
        self.status = AgentStatus::Initializing;
        let pool = create_pool(&self.config)
            .await
            .map_err(|e| chaos_core::error::ChaosError::Connection(e))?;
        self.pool = Some(pool);
        self.status = AgentStatus::Ready;
        tracing::info!(db_type = ?self.config.db_type, "Database agent initialized");
        Ok(())
    }

    async fn discover(&mut self) -> ChaosResult<Vec<Box<dyn DiscoveredResource>>> {
        self.status = AgentStatus::Discovering;
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized")))?;

        let resources = discover_schema(pool)
            .await
            .map_err(|e| chaos_core::error::ChaosError::Discovery(e.to_string()))?;

        tracing::info!(tables = resources.len(), "Schema discovery complete");
        self.status = AgentStatus::Ready;

        Ok(resources
            .into_iter()
            .map(|r| Box::new(r) as Box<dyn DiscoveredResource>)
            .collect())
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
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized")))?
            .clone();

        Ok(SkillContext {
            shared: Box::new(pool),
            params: serde_yaml::Value::Null,
        })
    }

    async fn shutdown(&mut self) -> ChaosResult<()> {
        if let Some(pool) = self.pool.take() {
            pool.close().await;
        }
        self.status = AgentStatus::Idle;
        tracing::info!("Database agent shut down");
        Ok(())
    }
}
