use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::any::AnyPool;
use sqlx::Row;

pub struct UpdateLoadSkill;

#[derive(Debug, Deserialize)]
struct UpdateParams {
    #[serde(default = "default_rows")]
    rows: u32,
    #[serde(default)]
    tables: Vec<String>,
}

fn default_rows() -> u32 {
    100
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateUndoEntry {
    table: String,
    schema: String,
    pk_column: String,
    pk_value: String,
    column: String,
    original_value: String,
}

#[async_trait]
impl Skill for UpdateLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.update_load".into(),
            description: "Randomly UPDATE existing rows in target tables".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: UpdateParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid update_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let params: UpdateParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let tables_to_target = if params.tables.is_empty() {
            let rows = sqlx::query(
                "SELECT table_schema, table_name FROM information_schema.tables \
                 WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'mysql', 'performance_schema', 'sys') \
                 AND table_type = 'BASE TABLE' LIMIT 5",
            )
            .fetch_all(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Failed to list tables: {e}")))?;

            rows.iter()
                .map(|r| {
                    let schema: String = r.get("table_schema");
                    let table: String = r.get("table_name");
                    (schema, table)
                })
                .collect::<Vec<_>>()
        } else {
            params
                .tables
                .iter()
                .map(|t| ("public".to_string(), t.clone()))
                .collect()
        };

        let mut all_undo = Vec::new();

        for (schema, table) in &tables_to_target {
            // Find PK and a text-like column to update
            let cols = sqlx::query(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
            )
            .bind(schema)
            .bind(table)
            .fetch_all(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Column fetch failed: {e}")))?;

            let pk_col = sqlx::query(
                "SELECT c.column_name FROM information_schema.columns c \
                 JOIN information_schema.key_column_usage kcu \
                   ON c.table_schema = kcu.table_schema AND c.table_name = kcu.table_name AND c.column_name = kcu.column_name \
                 JOIN information_schema.table_constraints tc \
                   ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' AND c.table_schema = $1 AND c.table_name = $2 LIMIT 1",
            )
            .bind(schema)
            .bind(table)
            .fetch_optional(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("PK fetch failed: {e}")))?;

            let pk_column: String = match pk_col {
                Some(row) => row.get("column_name"),
                None => continue,
            };

            // Find a text/varchar column to update
            let update_col = cols.iter().find(|c| {
                let dt: String = c.get("data_type");
                let name: String = c.get("column_name");
                name != pk_column
                    && (dt.contains("char") || dt.contains("text") || dt.contains("varchar"))
            });

            let update_column: String = match update_col {
                Some(c) => c.get("column_name"),
                None => continue,
            };

            // Fetch rows to update
            let query = format!(
                "SELECT {pk_column}, {update_column} FROM {schema}.{table} LIMIT $1"
            );
            let rows = sqlx::query(&query)
                .bind(params.rows as i64)
                .fetch_all(pool)
                .await;

            let rows = match rows {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(table = %table, error = %e, "Failed to fetch rows for update");
                    continue;
                }
            };

            for row in &rows {
                let pk_value: String = row
                    .try_get::<i64, _>(0)
                    .map(|v| v.to_string())
                    .or_else(|_| row.try_get::<i32, _>(0).map(|v| v.to_string()))
                    .or_else(|_| row.try_get::<String, _>(0))
                    .unwrap_or_default();

                let original: String = row.try_get::<String, _>(1).unwrap_or_default();

                let update_query = format!(
                    "UPDATE {schema}.{table} SET {update_column} = 'chaos_modified' WHERE {pk_column} = {pk_value}"
                );

                if sqlx::query(&update_query).execute(pool).await.is_ok() {
                    all_undo.push(UpdateUndoEntry {
                        table: table.clone(),
                        schema: schema.clone(),
                        pk_column: pk_column.clone(),
                        pk_value,
                        column: update_column.clone(),
                        original_value: original,
                    });
                }
            }

            tracing::info!(table = %table, updated = all_undo.len(), "Updated rows");
        }

        let undo_state = serde_yaml::to_value(&all_undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("db.update_load", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let entries: Vec<UpdateUndoEntry> = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for entry in &entries {
            let query = format!(
                "UPDATE {}.{} SET {} = '{}' WHERE {} = {}",
                entry.schema, entry.table, entry.column, entry.original_value, entry.pk_column, entry.pk_value
            );
            if let Err(e) = sqlx::query(&query).execute(pool).await {
                tracing::error!(table = %entry.table, error = %e, "Rollback update failed");
            }
        }

        Ok(())
    }
}
