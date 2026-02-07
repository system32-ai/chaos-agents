use async_trait::async_trait;
use mongodb::Client;

use chaos_core::agent::{Agent, AgentStatus};
use chaos_core::discovery::DiscoveredResource;
use chaos_core::error::ChaosResult;
use chaos_core::skill::{Skill, SkillContext, TargetDomain};

use crate::mongo_config::MongoTargetConfig;
use crate::mongo_discovery::discover_mongo;
use crate::skills::mongo_connection_stress::MongoConnectionStressSkill;
use crate::skills::mongo_find_load::MongoFindLoadSkill;
use crate::skills::mongo_index_drop::MongoIndexDropSkill;
use crate::skills::mongo_insert_load::MongoInsertLoadSkill;
use crate::skills::mongo_profiling_change::MongoProfilingChangeSkill;
use crate::skills::mongo_update_load::MongoUpdateLoadSkill;

pub struct MongoAgent {
    config: MongoTargetConfig,
    client: Option<Client>,
    status: AgentStatus,
    skills: Vec<Box<dyn Skill>>,
}

impl MongoAgent {
    pub fn new(config: MongoTargetConfig) -> Self {
        let skills: Vec<Box<dyn Skill>> = vec![
            Box::new(MongoInsertLoadSkill),
            Box::new(MongoUpdateLoadSkill),
            Box::new(MongoFindLoadSkill),
            Box::new(MongoIndexDropSkill),
            Box::new(MongoProfilingChangeSkill),
            Box::new(MongoConnectionStressSkill),
        ];
        Self {
            config,
            client: None,
            status: AgentStatus::Idle,
            skills,
        }
    }

    pub fn from_yaml(value: &serde_yaml::Value) -> ChaosResult<Self> {
        let config: MongoTargetConfig = serde_yaml::from_value(value.clone()).map_err(|e| {
            chaos_core::error::ChaosError::Config(format!("Invalid MongoDB config: {e}"))
        })?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl Agent for MongoAgent {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Database
    }

    fn name(&self) -> &str {
        "mongodb-chaos-agent"
    }

    fn status(&self) -> AgentStatus {
        self.status.clone()
    }

    async fn initialize(&mut self) -> ChaosResult<()> {
        self.status = AgentStatus::Initializing;
        let client = Client::with_uri_str(&self.config.connection_url)
            .await
            .map_err(|e| {
                chaos_core::error::ChaosError::Connection(anyhow::anyhow!(
                    "MongoDB connection failed: {e}"
                ))
            })?;

        // Verify connectivity by listing databases
        client.list_database_names().await.map_err(|e| {
            chaos_core::error::ChaosError::Connection(anyhow::anyhow!(
                "MongoDB ping failed: {e}"
            ))
        })?;

        self.client = Some(client);
        self.status = AgentStatus::Ready;
        tracing::info!("MongoDB agent initialized");
        Ok(())
    }

    async fn discover(&mut self) -> ChaosResult<Vec<Box<dyn DiscoveredResource>>> {
        self.status = AgentStatus::Discovering;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| {
                chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized"))
            })?;

        let resources = discover_mongo(client, &self.config.databases)
            .await
            .map_err(|e| chaos_core::error::ChaosError::Discovery(e.to_string()))?;

        tracing::info!(collections = resources.len(), "MongoDB discovery complete");
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
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| {
                chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized"))
            })?
            .clone();

        Ok(SkillContext {
            shared: Box::new(client),
            params: serde_yaml::Value::Null,
        })
    }

    async fn shutdown(&mut self) -> ChaosResult<()> {
        self.client = None;
        self.status = AgentStatus::Idle;
        tracing::info!("MongoDB agent shut down");
        Ok(())
    }
}
