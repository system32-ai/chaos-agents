use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use kube::Client;

use chaos_core::agent::{Agent, AgentStatus};
use chaos_core::discovery::{DiscoveredResource, K8sResource};
use chaos_core::error::ChaosResult;
use chaos_core::skill::{Skill, SkillContext, TargetDomain};

use crate::client::create_client;
use crate::config::K8sTargetConfig;
use crate::skills::network_chaos::NetworkChaosSkill;
use crate::skills::node_drain::NodeDrainSkill;
use crate::skills::pod_kill::PodKillSkill;
use crate::skills::resource_stress::ResourceStressSkill;

pub struct K8sAgent {
    config: K8sTargetConfig,
    client: Option<Client>,
    status: AgentStatus,
    skills: Vec<Box<dyn Skill>>,
}

impl K8sAgent {
    pub fn new(config: K8sTargetConfig) -> Self {
        let skills: Vec<Box<dyn Skill>> = vec![
            Box::new(PodKillSkill),
            Box::new(NodeDrainSkill),
            Box::new(NetworkChaosSkill),
            Box::new(ResourceStressSkill),
        ];
        Self {
            config,
            client: None,
            status: AgentStatus::Idle,
            skills,
        }
    }

    pub fn from_yaml(value: &serde_yaml::Value) -> ChaosResult<Self> {
        let config: K8sTargetConfig = serde_yaml::from_value(value.clone())
            .map_err(|e| chaos_core::error::ChaosError::Config(format!("Invalid K8s config: {e}")))?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl Agent for K8sAgent {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Kubernetes
    }

    fn name(&self) -> &str {
        "kubernetes-chaos-agent"
    }

    fn status(&self) -> AgentStatus {
        self.status.clone()
    }

    async fn initialize(&mut self) -> ChaosResult<()> {
        self.status = AgentStatus::Initializing;
        let client = create_client(&self.config)
            .await
            .map_err(chaos_core::error::ChaosError::Connection)?;
        self.client = Some(client);
        self.status = AgentStatus::Ready;
        tracing::info!(namespace = %self.config.namespace, "Kubernetes agent initialized");
        Ok(())
    }

    async fn discover(&mut self) -> ChaosResult<Vec<Box<dyn DiscoveredResource>>> {
        self.status = AgentStatus::Discovering;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized")))?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.config.namespace);
        let mut lp = ListParams::default();
        if let Some(ref selector) = self.config.label_selector {
            lp = lp.labels(selector);
        }

        let pod_list = pods
            .list(&lp)
            .await
            .map_err(|e| chaos_core::error::ChaosError::Discovery(format!("Pod list failed: {e}")))?;

        let resources: Vec<Box<dyn DiscoveredResource>> = pod_list
            .items
            .into_iter()
            .map(|p| {
                Box::new(K8sResource {
                    kind: "Pod".to_string(),
                    name: p.metadata.name.unwrap_or_default(),
                    namespace: p
                        .metadata
                        .namespace
                        .unwrap_or_else(|| self.config.namespace.clone()),
                    labels: p.metadata.labels.unwrap_or_default().into_iter().collect(),
                }) as Box<dyn DiscoveredResource>
            })
            .collect();

        tracing::info!(pods = resources.len(), "Kubernetes discovery complete");
        self.status = AgentStatus::Ready;

        Ok(resources)
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
            .ok_or_else(|| chaos_core::error::ChaosError::Connection(anyhow::anyhow!("Not initialized")))?
            .clone();

        Ok(SkillContext {
            shared: Box::new(client),
            params: serde_yaml::Value::Null,
        })
    }

    async fn shutdown(&mut self) -> ChaosResult<()> {
        self.client = None;
        self.status = AgentStatus::Idle;
        tracing::info!("Kubernetes agent shut down");
        Ok(())
    }
}
