use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, DeleteParams, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};

pub struct NetworkChaosSkill;

#[derive(Debug, Deserialize)]
struct NetworkChaosParams {
    #[serde(default = "default_namespace")]
    namespace: String,
    #[serde(default)]
    pod_selector: std::collections::BTreeMap<String, String>,
}

fn default_namespace() -> String {
    "default".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct NetworkChaosUndoState {
    policy_name: String,
    namespace: String,
}

#[async_trait]
impl Skill for NetworkChaosSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "k8s.network_chaos".into(),
            description: "Apply deny-all NetworkPolicy to isolate pods".into(),
            target: TargetDomain::Kubernetes,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: NetworkChaosParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid network_chaos params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let params: NetworkChaosParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let policy_name = format!("chaos-deny-{}", uuid::Uuid::new_v4().as_simple());

        // Create deny-all NetworkPolicy
        let policy: NetworkPolicy = serde_json::from_value(serde_json::json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {
                "name": policy_name,
                "namespace": params.namespace,
                "labels": {
                    "app.kubernetes.io/managed-by": "chaos-agents"
                }
            },
            "spec": {
                "podSelector": {
                    "matchLabels": params.pod_selector
                },
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [],
                "egress": []
            }
        }))
        .map_err(|e| ChaosError::Other(anyhow::anyhow!("Build NetworkPolicy: {e}")))?;

        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &params.namespace);
        np_api
            .create(&PostParams::default(), &policy)
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Failed to create NetworkPolicy: {e}"))
            })?;

        tracing::info!(
            policy = %policy_name,
            namespace = %params.namespace,
            "NetworkPolicy created (deny-all)"
        );

        let undo = NetworkChaosUndoState {
            policy_name,
            namespace: params.namespace,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("k8s.network_chaos", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let undo: NetworkChaosUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &undo.namespace);

        match np_api
            .delete(&undo.policy_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {
                tracing::info!(policy = %undo.policy_name, "NetworkPolicy deleted (rollback)");
            }
            Err(e) => {
                tracing::error!(policy = %undo.policy_name, error = %e, "Failed to delete NetworkPolicy");
            }
        }

        Ok(())
    }
}
