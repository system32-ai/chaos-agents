use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use mongodb::Client;
use serde::{Deserialize, Serialize};

pub struct MongoUpdateLoadSkill;

#[derive(Debug, Deserialize)]
struct UpdateParams {
    #[serde(default = "default_db")]
    database: String,
    #[serde(default)]
    collections: Vec<String>,
    #[serde(default = "default_docs")]
    docs: u32,
}

fn default_db() -> String {
    "test".to_string()
}

fn default_docs() -> u32 {
    100
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateUndoEntry {
    database: String,
    collection: String,
    id: String,
    original_doc: String,
}

#[async_trait]
impl Skill for MongoUpdateLoadSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.update_load".into(),
            description: "Randomly UPDATE existing documents in MongoDB collections".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: UpdateParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.update_load params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: UpdateParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let db = client.database(&params.database);

        // Discover collections if none specified
        let collections = if params.collections.is_empty() {
            db.list_collection_names()
                .await
                .map_err(|e| ChaosError::Discovery(format!("Failed to list collections: {e}")))?
                .into_iter()
                .filter(|c| !c.starts_with("system."))
                .take(5)
                .collect::<Vec<_>>()
        } else {
            params.collections.clone()
        };

        let mut all_undo = Vec::new();

        for coll_name in &collections {
            let coll = db.collection::<Document>(coll_name);

            // Fetch documents to update
            let mut cursor = coll
                .find(doc! {})
                .limit(params.docs as i64)
                .await
                .map_err(|e| {
                    ChaosError::Other(anyhow::anyhow!("Failed to query {coll_name}: {e}"))
                })?;

            let mut updated = 0u32;
            while let Some(original_doc) = cursor.try_next().await.map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Cursor error: {e}"))
            })? {
                let id = match original_doc.get("_id") {
                    Some(Bson::ObjectId(oid)) => *oid,
                    _ => continue,
                };

                // Save original for rollback
                let original_json = serde_json::to_string(&original_doc).unwrap_or_default();

                // Apply chaos modification
                let update = doc! {
                    "$set": {
                        "chaos_modified": true,
                        "chaos_modified_at": chrono::Utc::now().to_rfc3339(),
                    }
                };

                if coll
                    .update_one(doc! { "_id": id }, update)
                    .await
                    .is_ok()
                {
                    all_undo.push(UpdateUndoEntry {
                        database: params.database.clone(),
                        collection: coll_name.clone(),
                        id: id.to_hex(),
                        original_doc: original_json,
                    });
                    updated += 1;
                }
            }

            tracing::info!(collection = %coll_name, updated, "Updated documents");
        }

        let undo_state = serde_yaml::to_value(&all_undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("mongo.update_load", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let entries: Vec<UpdateUndoEntry> = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for entry in &entries {
            let db = client.database(&entry.database);
            let coll = db.collection::<Document>(&entry.collection);

            let oid = match ObjectId::parse_str(&entry.id) {
                Ok(o) => o,
                Err(_) => continue,
            };

            // Parse original document
            let original: Document = match serde_json::from_str(&entry.original_doc) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(id = %entry.id, error = %e, "Failed to parse original doc");
                    continue;
                }
            };

            // Replace document with original
            match coll
                .replace_one(doc! { "_id": oid }, original)
                .await
            {
                Ok(_) => {
                    tracing::info!(collection = %entry.collection, id = %entry.id, "Document restored");
                }
                Err(e) => {
                    tracing::error!(id = %entry.id, error = %e, "Rollback replace failed");
                }
            }
        }

        Ok(())
    }
}
