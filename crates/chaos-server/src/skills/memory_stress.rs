use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

pub struct MemoryStressSkill;

#[derive(Debug, Deserialize)]
struct MemoryStressParams {
    /// Amount of memory to consume, e.g. "512M", "1G"
    #[serde(default = "default_memory")]
    memory: String,
    #[serde(default = "default_workers")]
    workers: u32,
}

fn default_memory() -> String {
    "256M".to_string()
}
fn default_workers() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoryStressUndoState {
    host: String,
    pid_file: String,
}

#[async_trait]
impl Skill for MemoryStressSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "server.memory_stress".into(),
            description: "Run stress-ng to consume memory, rollback kills the process".into(),
            target: TargetDomain::Server,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: MemoryStressParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid memory_stress params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let params: MemoryStressParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let pid_file = format!(
            "/tmp/chaos-mem-stress-{}.pid",
            uuid::Uuid::new_v4().as_simple()
        );

        let cmd = format!(
            "nohup stress-ng --vm {} --vm-bytes {} --timeout 3600s > /dev/null 2>&1 & echo $! > {}",
            params.workers, params.memory, pid_file
        );

        let (exit_code, _, stderr) = ssh.exec(&cmd).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}"))
        })?;

        if exit_code != 0 {
            return Err(ChaosError::Other(anyhow::anyhow!(
                "Memory stress failed: {stderr}"
            )));
        }

        tracing::info!(
            host = %ssh.host,
            memory = %params.memory,
            "Memory stress started"
        );

        let undo = MemoryStressUndoState {
            host: ssh.host.clone(),
            pid_file,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("server.memory_stress", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let undo: MemoryStressUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let cmd = format!(
            "kill $(cat {} 2>/dev/null) 2>/dev/null; pkill -f 'stress-ng --vm' 2>/dev/null; rm -f {}",
            undo.pid_file, undo.pid_file
        );

        match ssh.exec(&cmd).await {
            Ok(_) => {
                tracing::info!(host = %undo.host, "Memory stress killed (rollback)");
            }
            Err(e) => {
                tracing::error!(host = %undo.host, error = %e, "Failed to kill memory stress");
            }
        }

        Ok(())
    }
}
