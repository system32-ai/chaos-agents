use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::Client;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

pub struct NodeDrainSkill;

#[derive(Debug, Deserialize)]
struct NodeDrainParams {
    #[serde(default)]
    node_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeDrainUndoState {
    nodes: Vec<String>,
}

#[async_trait]
impl Skill for NodeDrainSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "k8s.node_drain".into(),
            description: "Cordon a node (mark unschedulable), rollback uncordons it".into(),
            target: TargetDomain::Kubernetes,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: NodeDrainParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid node_drain params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let params: NodeDrainParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let nodes: Api<Node> = Api::all(client.clone());

        let target_node = if let Some(ref name) = params.node_name {
            name.clone()
        } else {
            // Pick a random schedulable worker node
            let node_list = nodes
                .list(&ListParams::default())
                .await
                .map_err(|e| ChaosError::Discovery(format!("Failed to list nodes: {e}")))?;

            let schedulable: Vec<_> = node_list
                .items
                .iter()
                .filter(|n| {
                    let unschedulable = n
                        .spec
                        .as_ref()
                        .and_then(|s| s.unschedulable)
                        .unwrap_or(false);
                    !unschedulable
                })
                .filter(|n| {
                    // Skip control plane nodes
                    let labels = n.metadata.labels.as_ref();
                    !labels
                        .map(|l| {
                            l.contains_key("node-role.kubernetes.io/control-plane")
                                || l.contains_key("node-role.kubernetes.io/master")
                        })
                        .unwrap_or(false)
                })
                .collect();

            if schedulable.is_empty() {
                return Err(ChaosError::Discovery(
                    "No schedulable worker nodes found".into(),
                ));
            }

            let mut rng = rand::thread_rng();
            schedulable
                .choose(&mut rng)
                .unwrap()
                .metadata
                .name
                .clone()
                .unwrap_or_default()
        };

        // Cordon: set spec.unschedulable = true
        let patch = serde_json::json!({
            "spec": {
                "unschedulable": true
            }
        });

        nodes
            .patch(
                &target_node,
                &PatchParams::apply("chaos-agents"),
                &Patch::Merge(&patch),
            )
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Failed to cordon node {target_node}: {e}"))
            })?;

        tracing::info!(node = %target_node, "Node cordoned (unschedulable)");

        let undo = NodeDrainUndoState {
            nodes: vec![target_node],
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("k8s.node_drain", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let undo: NodeDrainUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let nodes: Api<Node> = Api::all(client.clone());

        for node_name in &undo.nodes {
            let patch = serde_json::json!({
                "spec": {
                    "unschedulable": false
                }
            });

            match nodes
                .patch(
                    node_name,
                    &PatchParams::apply("chaos-agents"),
                    &Patch::Merge(&patch),
                )
                .await
            {
                Ok(_) => {
                    tracing::info!(node = %node_name, "Node uncordoned");
                }
                Err(e) => {
                    tracing::error!(node = %node_name, error = %e, "Failed to uncordon node");
                }
            }
        }

        Ok(())
    }
}
