use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use mongodb::Client;
use serde::{Deserialize, Serialize};

pub struct MongoInsertLoadSkill;

#[derive(Debug, Deserialize)]
struct InsertParams {
    #[serde(default = "default_db")]
    database: String,
    #[serde(default)]
    collections: Vec<String>,
    #[serde(default = "default_docs")]
    docs_per_collection: u32,
}

fn default_db() -> String {
    "test".to_string()
}

fn default_docs() -> u32 {
    1000
}

#[derive(Debug, Serialize, Deserialize)]
struct InsertUndoState {
    database: String,
    collection: String,
    inserted_ids: Vec<String>,
}

#[async_trait]
impl Skill for MongoInsertLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.insert_load".into(),
            description: "Bulk INSERT random documents into MongoDB collections".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: InsertParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.insert_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: InsertParams = serde_yaml::from_value(ctx.params.clone())
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

        let mut all_undo = Vec::new();

        for coll_name in &collections {
            let coll = db.collection::<Document>(coll_name);
            let mut inserted_ids = Vec::new();

            // Build batch of documents
            let mut docs = Vec::new();
            for i in 0..params.docs_per_collection {
                let doc = doc! {
                    "chaos_test": true,
                    "index": i as i64,
                    "data": format!("chaos_agent_test_{i}"),
                    "value": (i as f64) * 1.5,
                    "tags": ["chaos", "test"],
                    "nested": {
                        "field_a": format!("value_{i}"),
                        "field_b": i as i64 % 100,
                    }
                };
                docs.push(doc);
            }

            match coll.insert_many(&docs).await {
                Ok(result) => {
                    for (_, id) in &result.inserted_ids {
                        if let Bson::ObjectId(oid) = id {
                            inserted_ids.push(oid.to_hex());
                        }
                    }
                    tracing::info!(
                        collection = %coll_name,
                        count = inserted_ids.len(),
                        "Inserted documents"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        collection = %coll_name,
                        error = %e,
                        "Batch insert failed"
                    );
                }
            }

            if !inserted_ids.is_empty() {
                all_undo.push(InsertUndoState {
                    database: params.database.clone(),
                    collection: coll_name.clone(),
                    inserted_ids,
                });
            }
        }

        let undo_state = serde_yaml::to_value(&all_undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("mongo.insert_load", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let undo_states: Vec<InsertUndoState> =
            serde_yaml::from_value(handle.undo_state.clone())
                .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for undo in &undo_states {
            let db = client.database(&undo.database);
            let coll = db.collection::<Document>(&undo.collection);

            let oids: Vec<ObjectId> = undo
                .inserted_ids
                .iter()
                .filter_map(|id| ObjectId::parse_str(id).ok())
                .collect();

            if oids.is_empty() {
                continue;
            }

            let filter = doc! {
                "_id": { "$in": oids.iter().map(|o| Bson::ObjectId(*o)).collect::<Vec<_>>() }
            };

            match coll.delete_many(filter).await {
                Ok(result) => {
                    tracing::info!(
                        collection = %undo.collection,
                        deleted = result.deleted_count,
                        "Rollback: deleted inserted documents"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        collection = %undo.collection,
                        error = %e,
                        "Rollback delete failed"
                    );
                }
            }
        }

        Ok(())
    }
}
