use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::AnyPool;
use sqlx::Row;

/// YugabyteDB-specific: toggle follower reads and staleness settings.
/// When enabled, reads may be served from follower replicas with potential staleness,
/// testing how the application handles eventual consistency.
pub struct YsqlFollowerReadsSkill;

#[derive(Debug, Deserialize)]
struct FollowerReadsParams {
    /// Enable follower reads. Default: true (the chaos action).
    #[serde(default = "default_enable")]
    enable: bool,
    /// Max staleness duration, e.g. "30s", "1m". Default: "30s".
    #[serde(default = "default_staleness")]
    staleness: String,
}

fn default_enable() -> bool {
    true
}

fn default_staleness() -> String {
    "30000ms".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct FollowerReadsUndoState {
    original_follower_read: String,
    original_staleness: String,
}

#[async_trait]
impl Skill for YsqlFollowerReadsSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "ysql.follower_reads".into(),
            description: "Toggle YugabyteDB follower reads to test eventual consistency behavior".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: FollowerReadsParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid ysql.follower_reads params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let params: FollowerReadsParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        // Read current values
        let orig_follower = sqlx::query("SHOW yb_read_from_followers")
            .fetch_one(pool)
            .await
            .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
            .unwrap_or_else(|_| "off".to_string());

        let orig_staleness = sqlx::query("SHOW yb_follower_read_staleness_ms")
            .fetch_one(pool)
            .await
            .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
            .unwrap_or_else(|_| "30000".to_string());

        // Apply new settings
        let enable_str = if params.enable { "on" } else { "off" };

        sqlx::query(&format!(
            "SET yb_read_from_followers = '{}'",
            enable_str
        ))
        .execute(pool)
        .await
        .map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("Failed to set yb_read_from_followers: {e}"))
        })?;

        sqlx::query(&format!(
            "SET yb_follower_read_staleness_ms = '{}'",
            params.staleness
        ))
        .execute(pool)
        .await
        .map_err(|e| {
            ChaosError::Other(anyhow::anyhow!(
                "Failed to set yb_follower_read_staleness_ms: {e}"
            ))
        })?;

        tracing::info!(
            old_follower_reads = %orig_follower,
            new_follower_reads = %enable_str,
            staleness = %params.staleness,
            "YugabyteDB follower reads changed"
        );

        let undo = FollowerReadsUndoState {
            original_follower_read: orig_follower,
            original_staleness: orig_staleness,
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("ysql.follower_reads", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let undo: FollowerReadsUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        match sqlx::query(&format!(
            "SET yb_read_from_followers = '{}'",
            undo.original_follower_read
        ))
        .execute(pool)
        .await
        {
            Ok(_) => {
                tracing::info!(
                    value = %undo.original_follower_read,
                    "Rollback: yb_read_from_followers restored"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Rollback: failed to restore yb_read_from_followers");
            }
        }

        match sqlx::query(&format!(
            "SET yb_follower_read_staleness_ms = '{}'",
            undo.original_staleness
        ))
        .execute(pool)
        .await
        {
            Ok(_) => {
                tracing::info!(
                    value = %undo.original_staleness,
                    "Rollback: yb_follower_read_staleness_ms restored"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Rollback: failed to restore yb_follower_read_staleness_ms"
                );
            }
        }

        Ok(())
    }
}
