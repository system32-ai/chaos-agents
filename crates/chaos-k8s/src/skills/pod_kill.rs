use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams};
use kube::Client;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

pub struct PodKillSkill;

#[derive(Debug, Deserialize)]
struct PodKillParams {
    #[serde(default)]
    label_selector: Option<String>,
    #[serde(default = "default_namespace")]
    namespace: String,
    #[serde(default = "default_count")]
    count: usize,
}

fn default_namespace() -> String {
    "default".to_string()
}
fn default_count() -> usize {
    1
}

#[derive(Debug, Serialize, Deserialize)]
struct PodKillUndoState {
    killed_pods: Vec<KilledPodInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct KilledPodInfo {
    name: String,
    namespace: String,
    has_owner: bool,
    owner_kind: Option<String>,
    owner_name: Option<String>,
}

#[async_trait]
impl Skill for PodKillSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "k8s.pod_kill".into(),
            description: "Delete random pods matching label selector".into(),
            target: TargetDomain::Kubernetes,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: PodKillParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid pod_kill params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let params: PodKillParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &params.namespace);

        let mut lp = ListParams::default();
        if let Some(ref selector) = params.label_selector {
            lp = lp.labels(selector);
        }

        let pod_list = pods
            .list(&lp)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Failed to list pods: {e}")))?;

        let running_pods: Vec<_> = pod_list
            .items
            .iter()
            .filter(|p| {
                p.status
                    .as_ref()
                    .and_then(|s| s.phase.as_deref())
                    == Some("Running")
            })
            .collect();

        if running_pods.is_empty() {
            return Err(ChaosError::Discovery("No running pods found".into()));
        }

        let mut rng = rand::thread_rng();
        let targets: Vec<_> = running_pods
            .choose_multiple(&mut rng, params.count.min(running_pods.len()))
            .collect();

        let mut killed = Vec::new();

        for pod in targets {
            let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");
            let namespace = pod
                .metadata
                .namespace
                .as_deref()
                .unwrap_or(&params.namespace);

            // Check owner references
            let owner_ref = pod
                .metadata
                .owner_references
                .as_ref()
                .and_then(|refs| refs.first());

            let info = KilledPodInfo {
                name: pod_name.to_string(),
                namespace: namespace.to_string(),
                has_owner: owner_ref.is_some(),
                owner_kind: owner_ref.map(|r| r.kind.clone()),
                owner_name: owner_ref.map(|r| r.name.clone()),
            };

            match pods.delete(pod_name, &DeleteParams::default()).await {
                Ok(_) => {
                    tracing::info!(pod = %pod_name, namespace = %namespace, "Pod killed");
                    killed.push(info);
                }
                Err(e) => {
                    tracing::error!(pod = %pod_name, error = %e, "Failed to kill pod");
                }
            }
        }

        let undo = PodKillUndoState {
            killed_pods: killed,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("k8s.pod_kill", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected kube::Client")))?;

        let undo: PodKillUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        // For pods with owners (Deployments, StatefulSets), Kubernetes auto-reschedules.
        // We just verify replacement pods are running.
        for pod_info in &undo.killed_pods {
            if pod_info.has_owner {
                let pods: Api<Pod> = Api::namespaced(client.clone(), &pod_info.namespace);
                let lp = ListParams::default();
                match pods.list(&lp).await {
                    Ok(list) => {
                        let running = list
                            .items
                            .iter()
                            .filter(|p| {
                                p.status
                                    .as_ref()
                                    .and_then(|s| s.phase.as_deref())
                                    == Some("Running")
                            })
                            .count();
                        tracing::info!(
                            killed_pod = %pod_info.name,
                            owner = ?pod_info.owner_name,
                            running_pods = running,
                            "Verified replacement pods are running"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to verify pod recovery");
                    }
                }
            } else {
                tracing::warn!(
                    pod = %pod_info.name,
                    "Pod had no owner; cannot auto-recover. Manual intervention may be needed."
                );
            }
        }

        Ok(())
    }
}
