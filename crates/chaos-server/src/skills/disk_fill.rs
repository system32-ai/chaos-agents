use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

pub struct DiskFillSkill;

#[derive(Debug, Deserialize)]
struct DiskFillParams {
    #[serde(default = "default_size")]
    size: String,
    #[serde(default = "default_mount")]
    target_mount: String,
}

fn default_size() -> String {
    "1GB".to_string()
}
fn default_mount() -> String {
    "/tmp".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct DiskFillUndoState {
    host: String,
    file_path: String,
}

#[async_trait]
impl Skill for DiskFillSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "server.disk_fill".into(),
            description: "Fill disk space with a large file, rollback removes it".into(),
            target: TargetDomain::Server,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: DiskFillParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid disk_fill params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let params: DiskFillParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let file_id = uuid::Uuid::new_v4().as_simple().to_string();
        let file_path = format!("{}/chaos-agent-{}.fill", params.target_mount, file_id);

        let cmd = format!(
            "fallocate -l {} {} 2>/dev/null || dd if=/dev/zero of={} bs=1M count={} 2>/dev/null",
            params.size,
            file_path,
            file_path,
            parse_size_mb(&params.size)
        );

        let (exit_code, _stdout, stderr) = ssh.exec(&cmd).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}"))
        })?;

        if exit_code != 0 {
            return Err(ChaosError::Other(anyhow::anyhow!(
                "Disk fill failed (exit {}): {}",
                exit_code,
                stderr
            )));
        }

        tracing::info!(
            host = %ssh.host,
            path = %file_path,
            size = %params.size,
            "Disk filled"
        );

        let undo = DiskFillUndoState {
            host: ssh.host.clone(),
            file_path,
        };
        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("server.disk_fill", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let undo: DiskFillUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let cmd = format!("rm -f {}", undo.file_path);
        let (exit_code, _, stderr) = ssh.exec(&cmd).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}"))
        })?;

        if exit_code != 0 {
            tracing::error!(
                path = %undo.file_path,
                error = %stderr,
                "Failed to remove fill file"
            );
        } else {
            tracing::info!(path = %undo.file_path, "Fill file removed (rollback)");
        }

        Ok(())
    }
}

fn parse_size_mb(size: &str) -> u64 {
    let s = size.to_uppercase();
    if let Some(n) = s.strip_suffix("GB") {
        n.parse::<u64>().unwrap_or(1) * 1024
    } else if let Some(n) = s.strip_suffix("MB") {
        n.parse::<u64>().unwrap_or(100)
    } else if let Some(n) = s.strip_suffix("G") {
        n.parse::<u64>().unwrap_or(1) * 1024
    } else if let Some(n) = s.strip_suffix("M") {
        n.parse::<u64>().unwrap_or(100)
    } else {
        100
    }
}
