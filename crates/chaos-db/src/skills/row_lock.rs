use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::AnyPool;

use crate::config::DbType;
use crate::skills::lock_utils::{
    discover_user_tables, find_pk_column, get_backend_pid, terminate_backend,
    validate_row_lock_type,
};

pub struct RowLockSkill {
    pub db_type: DbType,
}

#[derive(Debug, Deserialize)]
struct RowLockParams {
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default = "default_rows_per_table")]
    rows_per_table: u32,
    #[serde(default = "default_lock_type")]
    lock_type: String,
}

fn default_rows_per_table() -> u32 {
    100
}

fn default_lock_type() -> String {
    "FOR UPDATE".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct RowLockUndoState {
    backend_pid: i32,
    locked_rows: Vec<LockedTableSummary>,
    lock_type: String,
    db_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockedTableSummary {
    table: String,
    schema: String,
    row_count: u32,
}

#[async_trait]
impl Skill for RowLockSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.row_lock".into(),
            description: "Acquire row-level locks (SELECT ... FOR UPDATE) to simulate row contention".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let p: RowLockParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid db.row_lock params: {e}")))?;
        validate_row_lock_type(&p.lock_type)?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let params: RowLockParams = serde_yaml::from_value(ctx.params.clone())
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

        // Begin transaction to scope the row locks
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("BEGIN failed: {e}")))?;

        let mut locked_rows = Vec::new();
        let lock_type_upper = params.lock_type.to_uppercase();

        for (schema, table) in &tables {
            let pk_col = match find_pk_column(&mut conn, schema, table).await {
                Some(col) => col,
                None => {
                    tracing::warn!(table = %table, "No primary key found, skipping row lock");
                    continue;
                }
            };

            let lock_sql = format!(
                "SELECT * FROM \"{schema}\".\"{table}\" ORDER BY \"{pk_col}\" LIMIT {} {lock_type_upper} NOWAIT",
                params.rows_per_table,
            );

            match sqlx::query(&lock_sql).fetch_all(&mut *conn).await {
                Ok(rows) => {
                    let count = rows.len() as u32;
                    tracing::info!(
                        table = %table,
                        rows_locked = count,
                        lock_type = %lock_type_upper,
                        "Row locks acquired"
                    );
                    locked_rows.push(LockedTableSummary {
                        table: table.clone(),
                        schema: schema.clone(),
                        row_count: count,
                    });
                }
                Err(e) => {
                    tracing::warn!(table = %table, error = %e, "Failed to lock rows, skipping");
                }
            }
        }

        if locked_rows.is_empty() {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            return Err(ChaosError::Other(anyhow::anyhow!(
                "No rows could be locked"
            )));
        }

        let backend_pid = get_backend_pid(&mut conn, self.db_type).await?;

        // Spawn a background task that holds the connection (and thus the row locks) alive
        tokio::spawn(async move {
            tracing::debug!(pid = backend_pid, "Row lock holder task started");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                // Periodic keepalive to prevent idle timeout
                match sqlx::query("SELECT 1").execute(&mut *conn).await {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::info!(
                            pid = backend_pid,
                            error = %e,
                            "Row lock holder connection terminated"
                        );
                        break;
                    }
                }
            }
        });

        let undo = RowLockUndoState {
            backend_pid,
            locked_rows: locked_rows.clone(),
            lock_type: lock_type_upper,
            db_type: format!("{:?}", self.db_type),
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        tracing::info!(
            pid = backend_pid,
            locked = ?locked_rows,
            "Row locks held by background connection"
        );

        Ok(RollbackHandle::new("db.row_lock", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let undo: RowLockUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        terminate_backend(pool, undo.backend_pid, &undo.db_type).await?;

        tracing::info!(
            pid = undo.backend_pid,
            locked = ?undo.locked_rows,
            "Row locks released via backend termination"
        );

        Ok(())
    }
}
