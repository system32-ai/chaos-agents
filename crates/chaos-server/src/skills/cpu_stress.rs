use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

pub struct CpuStressSkill;

#[derive(Debug, Deserialize)]
struct CpuStressParams {
    #[serde(default = "default_workers")]
    workers: u32,
}

fn default_workers() -> u32 {
    2
}

#[derive(Debug, Serialize, Deserialize)]
struct CpuStressUndoState {
    host: String,
    pid_file: String,
}

#[async_trait]
impl Skill for CpuStressSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "server.cpu_stress".into(),
            description: "Run stress-ng to load CPU, rollback kills the process".into(),
            target: TargetDomain::Server,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: CpuStressParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid cpu_stress params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let params: CpuStressParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let pid_file = format!("/tmp/chaos-cpu-stress-{}.pid", uuid::Uuid::new_v4().as_simple());

        // Start stress-ng in background, save PID
        let cmd = format!(
            "nohup stress-ng --cpu {} --timeout 3600s > /dev/null 2>&1 & echo $! > {}",
            params.workers, pid_file
        );

        let (exit_code, _, stderr) = ssh.exec(&cmd).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}"))
        })?;

        if exit_code != 0 {
            return Err(ChaosError::Other(anyhow::anyhow!(
                "CPU stress failed: {stderr}"
            )));
        }

        tracing::info!(
            host = %ssh.host,
            workers = params.workers,
            "CPU stress started"
        );

        let undo = CpuStressUndoState {
            host: ssh.host.clone(),
            pid_file,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("server.cpu_stress", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let undo: CpuStressUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        // Kill process and clean up
        let cmd = format!(
            "kill $(cat {} 2>/dev/null) 2>/dev/null; pkill -f 'stress-ng --cpu' 2>/dev/null; rm -f {}",
            undo.pid_file, undo.pid_file
        );

        match ssh.exec(&cmd).await {
            Ok(_) => {
                tracing::info!(host = %undo.host, "CPU stress killed (rollback)");
            }
            Err(e) => {
                tracing::error!(host = %undo.host, error = %e, "Failed to kill CPU stress");
            }
        }

        Ok(())
    }
}
