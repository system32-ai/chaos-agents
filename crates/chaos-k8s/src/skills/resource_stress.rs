use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};

pub struct ResourceStressSkill;

#[derive(Debug, Deserialize)]
struct ResourceStressParams {
    #[serde(default = "default_namespace")]
    namespace: String,
    /// CPU stress workers (number of stress-ng CPU workers)
    #[serde(default = "default_cpu_workers")]
    cpu_workers: u32,
    /// Memory to consume, e.g. "256M"
    #[serde(default = "default_memory")]
    memory: String,
    /// stress-ng image to use
    #[serde(default = "default_image")]
    image: String,
}

fn default_namespace() -> String {
    "default".to_string()
}
fn default_cpu_workers() -> u32 {
    2
}
fn default_memory() -> String {
    "256M".to_string()
}
fn default_image() -> String {
    "alexeiled/stress-ng:latest".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct ResourceStressUndoState {
    pod_name: String,
    namespace: String,
}

#[async_trait]
impl Skill for ResourceStressSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "k8s.resource_stress".into(),
            description: "Deploy a stress-ng pod to consume cluster resources".into(),
            target: TargetDomain::Kubernetes,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: ResourceStressParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid resource_stress params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let params: ResourceStressParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let pod_name = format!("chaos-stress-{}", uuid::Uuid::new_v4().as_simple());

        let stress_pod: Pod = serde_json::from_value(serde_json::json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": pod_name,
                "namespace": params.namespace,
                "labels": {
                    "app.kubernetes.io/managed-by": "chaos-agents",
                    "chaos-agents/type": "resource-stress"
                }
            },
            "spec": {
                "restartPolicy": "Never",
                "containers": [{
                    "name": "stress",
                    "image": params.image,
                    "command": [
                        "stress-ng",
                        "--cpu", params.cpu_workers.to_string(),
                        "--vm", "1",
                        "--vm-bytes", params.memory,
                        "--timeout", "3600s"
                    ]
                }]
            }
        }))
        .map_err(|e| ChaosError::Other(anyhow::anyhow!("Build stress pod: {e}")))?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &params.namespace);
        pods.create(&PostParams::default(), &stress_pod)
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Failed to create stress pod: {e}"))
            })?;

        tracing::info!(
            pod = %pod_name,
            cpu = params.cpu_workers,
            memory = %params.memory,
            "Stress pod deployed"
        );

        let undo = ResourceStressUndoState {
            pod_name,
            namespace: params.namespace,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("k8s.resource_stress", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let undo: ResourceStressUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &undo.namespace);

        match pods
            .delete(&undo.pod_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {
                tracing::info!(pod = %undo.pod_name, "Stress pod deleted (rollback)");
            }
            Err(e) => {
                tracing::error!(pod = %undo.pod_name, error = %e, "Failed to delete stress pod");
            }
        }

        Ok(())
    }
}
