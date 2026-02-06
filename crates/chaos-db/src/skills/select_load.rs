use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use serde::Deserialize;
use sqlx::any::AnyPool;
use sqlx::Row;

pub struct SelectLoadSkill;

#[derive(Debug, Deserialize)]
struct SelectParams {
    #[serde(default = "default_queries")]
    query_count: u32,
    #[serde(default)]
    tables: Vec<String>,
}

fn default_queries() -> u32 {
    500
}

#[async_trait]
impl Skill for SelectLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "db.select_load".into(),
            description: "Generate heavy SELECT query load against target tables".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: SelectParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid select_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let pool = ctx
            .shared
            .downcast_ref::<AnyPool>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected AnyPool")))?;

        let params: SelectParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let tables_to_target = if params.tables.is_empty() {
            let rows = sqlx::query(
                "SELECT table_schema, table_name FROM information_schema.tables \
                 WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'mysql', 'performance_schema', 'sys') \
                 AND table_type = 'BASE TABLE' LIMIT 10",
            )
            .fetch_all(pool)
            .await
            .map_err(|e| ChaosError::Discovery(format!("Table list failed: {e}")))?;

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

        let mut total_queries = 0u32;

        for (schema, table) in &tables_to_target {
            let per_table = params.query_count / tables_to_target.len().max(1) as u32;

            for _ in 0..per_table {
                // Run various heavy queries
                let queries = [
                    format!("SELECT * FROM {schema}.{table} ORDER BY random() LIMIT 100"),
                    format!("SELECT COUNT(*) FROM {schema}.{table}"),
                    format!(
                        "SELECT * FROM {schema}.{table} t1 CROSS JOIN (SELECT 1) t2 LIMIT 1000"
                    ),
                ];

                let q = &queries[total_queries as usize % queries.len()];
                match sqlx::query(q).fetch_all(pool).await {
                    Ok(_) => total_queries += 1,
                    Err(e) => {
                        tracing::debug!(error = %e, "Select query failed (expected for some query patterns)");
                        total_queries += 1;
                    }
                }
            }
        }

        tracing::info!(total_queries, "Select load completed");

        // Select load is read-only, no real rollback needed
        let undo_state = serde_yaml::to_value(serde_json::json!({
            "queries_executed": total_queries,
            "note": "read-only, no rollback needed"
        }))
        .unwrap_or(serde_yaml::Value::Null);

        Ok(RollbackHandle::new("db.select_load", undo_state))
    }

    async fn rollback(&self, _ctx: &SkillContext, _handle: &RollbackHandle) -> ChaosResult<()> {
        // Select load is read-only, nothing to rollback
        tracing::info!("Select load rollback: no-op (read-only)");
        Ok(())
    }
}
