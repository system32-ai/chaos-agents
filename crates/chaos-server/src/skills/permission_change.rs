use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

pub struct PermissionChangeSkill;

#[derive(Debug, Deserialize)]
struct PermissionParams {
    /// Paths to target. If empty, discovered service config paths are used.
    #[serde(default)]
    paths: Vec<String>,
    /// Permission to set (e.g. "000"). Defaults to "000".
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "000".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct PermissionUndoEntry {
    host: String,
    path: String,
    original_mode: String,
}

#[async_trait]
impl Skill for PermissionChangeSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "server.permission_change".into(),
            description: "Change file permissions to disrupt services, rollback restores them".into(),
            target: TargetDomain::Server,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: PermissionParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid permission_change params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let params: PermissionParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let paths = if params.paths.is_empty() {
            // Discover some config directories
            let (_, stdout, _) = ssh
                .exec("ls -d /etc/nginx /etc/mysql /etc/postgresql /etc/redis /etc/apache2 /etc/httpd 2>/dev/null || true")
                .await
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}")))?;

            stdout.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
        } else {
            params.paths.clone()
        };

        if paths.is_empty() {
            return Err(ChaosError::Discovery(
                "No target paths found for permission change".into(),
            ));
        }

        let mut undo_entries = Vec::new();

        for path in &paths {
            // Capture original permissions
            let (exit_code, stdout, _) = ssh
                .exec(&format!("stat -c '%a' {} 2>/dev/null || stat -f '%Lp' {} 2>/dev/null", path, path))
                .await
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}")))?;

            if exit_code != 0 || stdout.trim().is_empty() {
                tracing::warn!(path = %path, "Could not read permissions, skipping");
                continue;
            }

            let original_mode = stdout.trim().to_string();

            // Change permissions
            let (exit_code, _, stderr) = ssh
                .exec(&format!("chmod {} {}", params.mode, path))
                .await
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("SSH exec failed: {e}")))?;

            if exit_code != 0 {
                tracing::warn!(path = %path, error = %stderr, "chmod failed");
                continue;
            }

            tracing::info!(
                host = %ssh.host,
                path = %path,
                old_mode = %original_mode,
                new_mode = %params.mode,
                "Permissions changed"
            );

            undo_entries.push(PermissionUndoEntry {
                host: ssh.host.clone(),
                path: path.clone(),
                original_mode,
            });
        }

        let undo_state = serde_yaml::to_value(&undo_entries)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("server.permission_change", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let ssh = ctx
            .shared
            .downcast_ref::<SshSession>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected SshSession")))?;

        let entries: Vec<PermissionUndoEntry> =
            serde_yaml::from_value(handle.undo_state.clone())
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for entry in &entries {
            let cmd = format!("chmod {} {}", entry.original_mode, entry.path);
            match ssh.exec(&cmd).await {
                Ok((0, _, _)) => {
                    tracing::info!(path = %entry.path, mode = %entry.original_mode, "Permissions restored");
                }
                Ok((code, _, stderr)) => {
                    tracing::error!(path = %entry.path, exit_code = code, error = %stderr, "Permission restore failed");
                }
                Err(e) => {
                    tracing::error!(path = %entry.path, error = %e, "SSH failed during rollback");
                }
            }
        }

        Ok(())
    }
}
