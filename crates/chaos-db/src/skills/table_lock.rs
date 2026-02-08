use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::AnyPool;

use crate::config::DbType;
use crate::skills::lock_utils::{
    discover_user_tables, get_backend_pid, terminate_backend, validate_lock_mode,
};

pub struct TableLockSkill {
    pub db_type: DbType,
}

#[derive(Debug, Deserialize)]
struct TableLockParams {
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default = "default_lock_mode")]
    lock_mode: String,
}

fn default_lock_mode() -> String {
    "ACCESS EXCLUSIVE".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct TableLockUndoState {
    backend_pid: i32,
    locked_tables: Vec<String>,
    lock_mode: String,
    db_type: String,
}

#[async_trait]
impl Skill for TableLockSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.table_lock".into(),
            description: "Acquire table-level locks to simulate lock contention".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let p: TableLockParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid db.table_lock params: {e}")))?;
        validate_lock_mode(&p.lock_mode)?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let params: TableLockParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let tables = if params.tables.is_empty() {
            discover_user_tables(pool).await?
        } else {
            params
                .tables
                .iter()
                .map(|t| ("public".to_string(), t.clone()))
                .collect()
        };

        // Acquire a dedicated connection and hold it for the lock duration
        let mut conn = pool.acquire().await.map_err(|e| {
            ChaosError::Connection(anyhow::anyhow!("Failed to acquire connection: {e}"))
        })?;

        // Begin transaction to scope the locks
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("BEGIN failed: {e}")))?;

        let mut locked_tables = Vec::new();
        let lock_mode_upper = params.lock_mode.to_uppercase();

        for (schema, table) in &tables {
            let lock_sql = if self.db_type == DbType::Mysql {
                let mysql_mode = if lock_mode_upper.contains("EXCLUSIVE") {
                    "WRITE"
                } else {
                    "READ"
                };
                format!("LOCK TABLES `{table}` {mysql_mode}")
            } else {
                format!(
                    "LOCK TABLE \"{schema}\".\"{table}\" IN {lock_mode_upper} MODE NOWAIT"
                )
            };

            match sqlx::query(&lock_sql).execute(&mut *conn).await {
                Ok(_) => {
                    tracing::info!(table = %table, mode = %lock_mode_upper, "Table lock acquired");
                    locked_tables.push(format!("{schema}.{table}"));
                }
                Err(e) => {
                    tracing::warn!(table = %table, error = %e, "Failed to lock table, skipping");
                }
            }
        }

        if locked_tables.is_empty() {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            return Err(ChaosError::Other(anyhow::anyhow!(
                "No tables could be locked"
            )));
        }

        let backend_pid = get_backend_pid(&mut conn, self.db_type).await?;

        // Spawn a background task that holds the connection (and thus the locks) alive
        tokio::spawn(async move {
            tracing::debug!(pid = backend_pid, "Table lock holder task started");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                // Periodic keepalive to prevent idle timeout
                match sqlx::query("SELECT 1").execute(&mut *conn).await {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::info!(
                            pid = backend_pid,
                            error = %e,
                            "Table lock holder connection terminated"
                        );
                        break;
                    }
                }
            }
        });

        let undo = TableLockUndoState {
            backend_pid,
            locked_tables: locked_tables.clone(),
            lock_mode: lock_mode_upper,
            db_type: format!("{:?}", self.db_type),
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        tracing::info!(
            pid = backend_pid,
            tables = ?locked_tables,
            "Table locks held by background connection"
        );

        Ok(RollbackHandle::new("db.table_lock", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let undo: TableLockUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        terminate_backend(pool, undo.backend_pid, &undo.db_type).await?;

        tracing::info!(
            pid = undo.backend_pid,
            tables = ?undo.locked_tables,
            "Table locks released via backend termination"
        );

        Ok(())
    }
}
