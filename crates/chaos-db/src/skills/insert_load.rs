use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::{Deserialize, Serialize};
use sqlx::any::AnyPool;
use sqlx::Row;

pub struct InsertLoadSkill;

#[derive(Debug, Deserialize)]
struct InsertParams {
    #[serde(default = "default_rows")]
    rows_per_table: u32,
    #[serde(default)]
    tables: Vec<String>,
}

fn default_rows() -> u32 {
    1000
}

#[derive(Debug, Serialize, Deserialize)]
struct InsertUndoState {
    table: String,
    schema: String,
    pk_column: String,
    inserted_ids: Vec<String>,
}

#[async_trait]
impl Skill for InsertLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.insert_load".into(),
            description: "Bulk INSERT random rows into target tables".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: InsertParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid insert_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let params: InsertParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        // Discover tables if none specified
        let tables_to_target = if params.tables.is_empty() {
            let rows = sqlx::query(
                "SELECT table_schema, table_name FROM information_schema.tables \
                 WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'mysql', 'performance_schema', 'sys') \
                 AND table_type = 'BASE TABLE' LIMIT 10"
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
            // Find the primary key column
            let pk_row = sqlx::query(
                "SELECT c.column_name FROM information_schema.columns c \
                 JOIN information_schema.key_column_usage kcu \
                   ON c.table_schema = kcu.table_schema AND c.table_name = kcu.table_name AND c.column_name = kcu.column_name \
                 JOIN information_schema.table_constraints tc \
                   ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' AND c.table_schema = $1 AND c.table_name = $2 \
                 LIMIT 1",
            )
            .bind(schema)
            .bind(table)
            .fetch_optional(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Failed to find PK for {table}: {e}")))?;

            let pk_column: String = match pk_row {
                Some(row) => row.get("column_name"),
                None => {
                    tracing::warn!(table = %table, "No primary key found, skipping");
                    continue;
                }
            };

            // Get column info for generating data
            let columns = sqlx::query(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 \
                 AND column_name != $3 \
                 ORDER BY ordinal_position",
            )
            .bind(schema)
            .bind(table)
            .bind(&pk_column)
            .fetch_all(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Failed to get columns: {e}")))?;

            let col_names: Vec<String> = columns.iter().map(|c| c.get("column_name")).collect();
            let col_types: Vec<String> = columns.iter().map(|c| c.get("data_type")).collect();

            if col_names.is_empty() {
                tracing::warn!(table = %table, "No non-PK columns found, skipping");
                continue;
            }

            let mut inserted_ids = Vec::new();

            for i in 0..params.rows_per_table {
                let values: Vec<String> = col_types
                    .iter()
                    .map(|dt| generate_value(dt, i))
                    .collect();

                let col_list = col_names.join(", ");
                let val_list = values.join(", ");
                let query = format!(
                    "INSERT INTO {schema}.{table} ({col_list}) VALUES ({val_list}) RETURNING {pk_column}"
                );

                match sqlx::query(&query).fetch_one(pool).await {
                    Ok(row) => {
                        // Try to get the ID as a string
                        let id: String = row
                            .try_get::<i64, _>(0)
                            .map(|v| v.to_string())
                            .or_else(|_| row.try_get::<i32, _>(0).map(|v| v.to_string()))
                            .or_else(|_| row.try_get::<String, _>(0))
                            .unwrap_or_else(|_| format!("unknown_{i}"));
                        inserted_ids.push(id);
                    }
                    Err(e) => {
                        tracing::warn!(table = %table, error = %e, "Insert failed, stopping load for this table");
                        break;
                    }
                }
            }

            if !inserted_ids.is_empty() {
                tracing::info!(table = %table, count = inserted_ids.len(), "Inserted rows");
                all_undo.push(InsertUndoState {
                    table: table.clone(),
                    schema: schema.clone(),
                    pk_column: pk_column.clone(),
                    inserted_ids,
                });
            }
        }

        let undo_state = serde_yaml::to_value(&all_undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Failed to serialize undo state: {e}")))?;

        Ok(RollbackHandle::new("db.insert_load", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool in context")))?;

        let undo_states: Vec<InsertUndoState> = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Failed to parse undo state: {e}")))?;

        for undo in &undo_states {
            let id_list = undo.inserted_ids.join(", ");
            let query = format!(
                "DELETE FROM {}.{} WHERE {} IN ({})",
                undo.schema, undo.table, undo.pk_column, id_list
            );
            match sqlx::query(&query).execute(pool).await {
                Ok(result) => {
                    tracing::info!(
                        table = %undo.table,
                        deleted = result.rows_affected(),
                        "Rollback: deleted inserted rows"
                    );
                }
                Err(e) => {
                    tracing::error!(table = %undo.table, error = %e, "Rollback delete failed");
                }
            }
        }

        Ok(())
    }
}

fn generate_value(data_type: &str, seed: u32) -> String {
    let dt = data_type.to_lowercase();
    if dt.contains("int") || dt.contains("serial") {
        format!("{}", seed + 1000)
    } else if dt.contains("float") || dt.contains("double") || dt.contains("numeric") || dt.contains("decimal") {
        format!("{}.{}", seed, seed % 100)
    } else if dt.contains("bool") {
        if seed % 2 == 0 { "true".into() } else { "false".into() }
    } else if dt.contains("timestamp") || dt.contains("datetime") {
        "'2024-01-01 00:00:00'".into()
    } else if dt.contains("date") {
        "'2024-01-01'".into()
    } else if dt.contains("json") {
        format!("'{}'", serde_json::json!({"chaos": seed}))
    } else {
        // Default to text/varchar
        format!("'chaos_agent_test_{seed}'")
    }
}
