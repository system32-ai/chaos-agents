use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::AnyPool;
use sqlx::Row;

/// CockroachDB-specific: change zone configuration for databases or tables.
/// This controls replication factor, GC TTL, and range sizes.
pub struct CrdbZoneConfigSkill;

#[derive(Debug, Deserialize)]
struct ZoneConfigParams {
    /// Target: "DATABASE dbname" or "TABLE schema.table" or "RANGE default"
    target: String,
    /// Zone config overrides
    changes: Vec<ZoneConfigEntry>,
}

#[derive(Debug, Deserialize)]
struct ZoneConfigEntry {
    /// e.g. "num_replicas", "gc.ttlseconds", "range_min_bytes"
    param: String,
    /// e.g. "1", "3600", "134217728"
    value: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ZoneConfigUndoState {
    target: String,
    /// The full original zone config YAML from SHOW ZONE CONFIGURATION
    original_config: String,
}

#[async_trait]
impl Skill for CrdbZoneConfigSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "crdb.zone_config_change".into(),
            description: "Change CockroachDB zone configuration (replication factor, GC TTL, range sizes)".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: ZoneConfigParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid crdb.zone_config_change params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let params: ZoneConfigParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        // Get the current zone configuration for rollback
        let show_query = format!("SHOW ZONE CONFIGURATION FOR {}", params.target);
        let current = sqlx::query(&show_query)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!(
                    "Failed to read zone config for {}: {e}",
                    params.target
                ))
            })?;

        let original_config = current
            .as_ref()
            .and_then(|row| row.try_get::<String, _>("raw_config_sql").ok())
            .unwrap_or_default();

        // Apply new zone config
        let overrides: Vec<String> = params
            .changes
            .iter()
            .map(|c| format!("{} = {}", c.param, c.value))
            .collect();

        let alter_query = format!(
            "ALTER {} CONFIGURE ZONE USING {}",
            params.target,
            overrides.join(", ")
        );

        sqlx::query(&alter_query)
            .execute(pool)
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!(
                    "Failed to alter zone config for {}: {e}",
                    params.target
                ))
            })?;

        for change in &params.changes {
            tracing::info!(
                target = %params.target,
                param = %change.param,
                value = %change.value,
                "Zone config changed"
            );
        }

        let undo = ZoneConfigUndoState {
            target: params.target,
            original_config,
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("crdb.zone_config_change", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let undo: ZoneConfigUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        if undo.original_config.is_empty() {
            // No previous config — discard overrides to restore defaults
            let discard_query = format!("ALTER {} CONFIGURE ZONE DISCARD", undo.target);
            match sqlx::query(&discard_query).execute(pool).await {
                Ok(_) => {
                    tracing::info!(target = %undo.target, "Rollback: zone config reset to defaults");
                }
                Err(e) => {
                    tracing::error!(target = %undo.target, error = %e, "Rollback: failed to discard zone config");
                }
            }
        } else {
            // Re-apply the original configuration
            match sqlx::query(&undo.original_config).execute(pool).await {
                Ok(_) => {
                    tracing::info!(target = %undo.target, "Rollback: zone config restored");
                }
                Err(e) => {
                    tracing::error!(target = %undo.target, error = %e, "Rollback: failed to restore zone config");
                }
            }
        }

        Ok(())
    }
}
