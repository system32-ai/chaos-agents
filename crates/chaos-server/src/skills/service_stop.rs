use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

pub struct ServiceStopSkill;

#[derive(Debug, Deserialize)]
struct ServiceStopParams {
    /// Max number of services to stop. If 0, stop one random service.
    #[serde(default = "default_max")]
    max_services: usize,
    /// Specific services to stop. If empty, picks from discovered services.
    #[serde(default)]
    services: Vec<String>,
}

fn default_max() -> usize {
    1
}

#[derive(Debug, Serialize, Deserialize)]
struct ServiceStopUndoState {
    stopped_services: Vec<StoppedService>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoppedService {
    host: String,
    service_name: String,
}

#[async_trait]
impl Skill for ServiceStopSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "server.service_stop".into(),
            description: "Stop random running services, rollback restarts them".into(),
            target: TargetDomain::Server,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: ServiceStopParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid service_stop params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let params: ServiceStopParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let services_to_stop = if params.services.is_empty() {
            // Discover services and pick random ones
            let (_, stdout, _) = ssh
                .exec("systemctl list-units --type=service --state=running --no-legend --plain 2>/dev/null || true")
                .await
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}")))?;

            let excluded = [
                "sshd", "ssh", "systemd", "dbus", "NetworkManager", "network",
                "firewalld", "iptables", "ufw", "chaos",
            ];

            let available: Vec<String> = stdout
                .lines()
                .filter_map(|line| {
                    let name = line.split_whitespace().next()?;
                    let name = name.trim_end_matches(".service");
                    if excluded.iter().any(|&e| name.contains(e)) {
                        None
                    } else {
                        Some(name.to_string())
                    }
                })
                .collect();

            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            available
                .choose_multiple(&mut rng, params.max_services.min(available.len()))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            params.services.clone()
        };

        if services_to_stop.is_empty() {
            return Err(ChaosError::Discovery(
                "No eligible services found to stop".into(),
            ));
        }

        let mut stopped = Vec::new();

        for service in &services_to_stop {
            let cmd = format!("systemctl stop {service}");
            let (exit_code, _, stderr) = ssh.exec(&cmd).await.map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}"))
            })?;

            if exit_code != 0 {
                tracing::warn!(service = %service, error = %stderr, "Failed to stop service");
                continue;
            }

            tracing::info!(host = %ssh.host, service = %service, "Service stopped");
            stopped.push(StoppedService {
                host: ssh.host.clone(),
                service_name: service.clone(),
            });
        }

        let undo = ServiceStopUndoState {
            stopped_services: stopped,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("server.service_stop", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let undo: ServiceStopUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for svc in &undo.stopped_services {
            let cmd = format!("systemctl start {}", svc.service_name);
            match ssh.exec(&cmd).await {
                Ok((0, _, _)) => {
                    tracing::info!(service = %svc.service_name, "Service restarted (rollback)");
                }
                Ok((code, _, stderr)) => {
                    tracing::error!(
                        service = %svc.service_name,
                        exit_code = code,
                        error = %stderr,
                        "Failed to restart service"
                    );
                }
                Err(e) => {
                    tracing::error!(service = %svc.service_name, error = %e, "SSH failed during rollback");
                }
            }
        }

        Ok(())
    }
}
