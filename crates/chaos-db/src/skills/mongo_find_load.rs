use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use mongodb::Client;
use serde::Deserialize;

pub struct MongoFindLoadSkill;

#[derive(Debug, Deserialize)]
struct FindParams {
    #[serde(default = "default_db")]
    database: String,
    #[serde(default)]
    collections: Vec<String>,
    #[serde(default = "default_queries")]
    query_count: u32,
}

fn default_db() -> String {
    "test".to_string()
}

fn default_queries() -> u32 {
    500
}

#[async_trait]
impl Skill for MongoFindLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.find_load".into(),
            description: "Generate heavy read (find) query load against MongoDB collections".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: FindParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.find_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: FindParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let db = client.database(&params.database);

        // Discover collections if none specified
        let collections = if params.collections.is_empty() {
            db.list_collection_names()
                .await
                .map_err(|e| ChaosError::Discovery(format!("Failed to list collections: {e}")))?
                .into_iter()
                .filter(|c| !c.starts_with("system."))
                .take(10)
                .collect::<Vec<_>>()
        } else {
            params.collections.clone()
        };

        let mut total_queries = 0u32;

        for coll_name in &collections {
            let coll = db.collection::<Document>(coll_name);
            let per_coll = params.query_count / collections.len().max(1) as u32;

            for i in 0..per_coll {
                let query_result = match i % 4 {
                    // Full collection scan with limit
                    0 => coll.find(doc! {}).limit(100).await,
                    // Count documents
                    1 => {
                        let _ = coll.count_documents(doc! {}).await;
                        total_queries += 1;
                        continue;
                    }
                    // Filter query
                    2 => coll.find(doc! { "chaos_test": true }).limit(100).await,
                    // Aggregation pipeline
                    _ => {
                        let pipeline = vec![
                            doc! { "$sample": { "size": 100 } },
                            doc! { "$group": { "_id": null, "count": { "$sum": 1 } } },
                        ];
                        match coll.aggregate(pipeline).await {
                            Ok(mut cursor) => {
                                while cursor.try_next().await.ok().flatten().is_some() {}
                                total_queries += 1;
                                continue;
                            }
                            Err(e) => {
                                tracing::debug!(error = %e, "Aggregation failed");
                                total_queries += 1;
                                continue;
                            }
                        }
                    }
                };

                match query_result {
                    Ok(mut cursor) => {
                        while cursor.try_next().await.ok().flatten().is_some() {}
                        total_queries += 1;
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "Find query failed");
                        total_queries += 1;
                    }
                }
            }
        }

        tracing::info!(total_queries, "MongoDB find load completed");

        let undo_state = serde_yaml::to_value(serde_json::json!({
            "queries_executed": total_queries,
            "note": "read-only, no rollback needed"
        }))
        .unwrap_or(serde_yaml::Value::Null);

        Ok(RollbackHandle::new("mongo.find_load", undo_state))
    }

    async fn rollback(&self, _ctx: &SkillContext, _handle: &RollbackHandle) -> ChaosResult<()> {
        tracing::info!("MongoDB find load rollback: no-op (read-only)");
        Ok(())
    }
}
