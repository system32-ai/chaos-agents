use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::AnyPool;
use sqlx::Row;

use crate::config::DbType;

pub struct ConfigChangeSkill {
    pub db_type: DbType,
}

#[derive(Debug, Deserialize)]
struct ConfigChangeParams {
    changes: Vec<ConfigEntry>,
}

#[derive(Debug, Deserialize)]
struct ConfigEntry {
    param: String,
    value: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigUndoEntry {
    param: String,
    original_value: String,
    db_type: String,
}

#[async_trait]
impl Skill for ConfigChangeSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.config_change".into(),
            description: "ALTER database configuration parameters with rollback".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: ConfigChangeParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid config_change params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let params: ConfigChangeParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let mut undo_entries = Vec::new();

        for change in &params.changes {
            // Get current value
            let original_value = match self.db_type {
                DbType::Postgres | DbType::YugabyteDb => {
                    let query = format!("SHOW {}", change.param);
                    let row = sqlx::query(&query)
                        .fetch_one(pool)
                        .await
                        .map_err(|e| {
                            ChaosError::Other(anyhow::anyhow!(
                                "Failed to read config {}: {e}",
                                change.param
                            ))
                        })?;
                    row.try_get::<String, _>(0).unwrap_or_default()
                }
                DbType::CockroachDb => {
                    let query = format!("SHOW CLUSTER SETTING {}", change.param);
                    let row = sqlx::query(&query)
                        .fetch_one(pool)
                        .await
                        .map_err(|e| {
                            ChaosError::Other(anyhow::anyhow!(
                                "Failed to read cluster setting {}: {e}",
                                change.param
                            ))
                        })?;
                    row.try_get::<String, _>(0).unwrap_or_default()
                }
                DbType::Mysql => {
                    let query = format!("SELECT @@{}", change.param);
                    let row = sqlx::query(&query)
                        .fetch_one(pool)
                        .await
                        .map_err(|e| {
                            ChaosError::Other(anyhow::anyhow!(
                                "Failed to read config {}: {e}",
                                change.param
                            ))
                        })?;
                    row.try_get::<String, _>(0).unwrap_or_default()
                }
                DbType::MongoDB => {
                    return Err(ChaosError::Config(
                        "config_change skill not supported for MongoDB; use mongo-specific skills"
                            .into(),
                    ));
                }
            };

            // Apply new value
            let alter_query = match self.db_type {
                DbType::Postgres | DbType::YugabyteDb => {
                    format!("ALTER SYSTEM SET {} = '{}'", change.param, change.value)
                }
                DbType::CockroachDb => {
                    format!("SET CLUSTER SETTING {} = '{}'", change.param, change.value)
                }
                DbType::Mysql => {
                    format!("SET GLOBAL {} = '{}'", change.param, change.value)
                }
                DbType::MongoDB => unreachable!(),
            };

            sqlx::query(&alter_query)
                .execute(pool)
                .await
                .map_err(|e| {
                    ChaosError::Other(anyhow::anyhow!(
                        "Failed to set {}: {e}",
                        change.param
                    ))
                })?;

            // For PostgreSQL-compatible, reload config
            if matches!(self.db_type, DbType::Postgres | DbType::YugabyteDb) {
                let _ = sqlx::query("SELECT pg_reload_conf()").execute(pool).await;
            }

            tracing::info!(
                param = %change.param,
                old = %original_value,
                new = %change.value,
                "Config changed"
            );

            undo_entries.push(ConfigUndoEntry {
                param: change.param.clone(),
                original_value,
                db_type: format!("{:?}", self.db_type),
            });
        }

        let undo_state = serde_yaml::to_value(&undo_entries)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("db.config_change", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let entries: Vec<ConfigUndoEntry> = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for entry in &entries {
            let db_lower = entry.db_type.to_lowercase();
            let restore_query = if db_lower.contains("cockroach") {
                format!(
                    "SET CLUSTER SETTING {} = '{}'",
                    entry.param, entry.original_value
                )
            } else if db_lower.contains("postgres") || db_lower.contains("yugabyte") {
                format!(
                    "ALTER SYSTEM SET {} = '{}'",
                    entry.param, entry.original_value
                )
            } else {
                format!(
                    "SET GLOBAL {} = '{}'",
                    entry.param, entry.original_value
                )
            };

            match sqlx::query(&restore_query).execute(pool).await {
                Ok(_) => {
                    tracing::info!(param = %entry.param, value = %entry.original_value, "Config restored");
                }
                Err(e) => {
                    tracing::error!(param = %entry.param, error = %e, "Config restore failed");
                }
            }

            // Reload for PostgreSQL-compatible
            if db_lower.contains("postgres") || db_lower.contains("yugabyte") {
                let _ = sqlx::query("SELECT pg_reload_conf()").execute(pool).await;
            }
        }

        Ok(())
    }
}
