use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use mongodb::bson::{doc, Document};
use mongodb::Client;
use serde::{Deserialize, Serialize};

pub struct MongoIndexDropSkill;

#[derive(Debug, Deserialize)]
struct IndexDropParams {
    #[serde(default = "default_db")]
    database: String,
    #[serde(default)]
    collections: Vec<String>,
    /// Max number of indexes to drop per collection. Default: 3.
    #[serde(default = "default_max_drops")]
    max_per_collection: usize,
}

fn default_db() -> String {
    "test".to_string()
}

fn default_max_drops() -> usize {
    3
}

#[derive(Debug, Serialize, Deserialize)]
struct IndexUndoEntry {
    database: String,
    collection: String,
    index_name: String,
    /// The key specification, e.g. {"field": 1, "other": -1}
    key: String,
    /// Whether it was unique
    unique: bool,
    /// Whether it was sparse
    sparse: bool,
    /// Optional TTL seconds
    expire_after_seconds: Option<i64>,
}

#[async_trait]
impl Skill for MongoIndexDropSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.index_drop".into(),
            description: "Drop secondary indexes from MongoDB collections to degrade query performance".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: IndexDropParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.index_drop params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: IndexDropParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let db = client.database(&params.database);

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

            // List indexes
            let mut cursor = coll.list_indexes().await.map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Failed to list indexes on {coll_name}: {e}"))
            })?;

            let mut droppable = Vec::new();
            use futures::TryStreamExt;
            while let Some(index_model) = cursor.try_next().await.map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Index cursor error: {e}"))
            })? {
                let opts = index_model.options.as_ref();
                let name = opts
                    .and_then(|o| o.name.as_deref())
                    .unwrap_or("")
                    .to_string();

                // Skip the _id index — it can't be dropped
                if name == "_id_" || name.is_empty() {
                    continue;
                }

                let key_doc = index_model.keys;
                let unique = opts.and_then(|o| o.unique).unwrap_or(false);
                let sparse = opts.and_then(|o| o.sparse).unwrap_or(false);
                let expire = opts.and_then(|o| o.expire_after).map(|d| d.as_secs() as i64);

                droppable.push(IndexUndoEntry {
                    database: params.database.clone(),
                    collection: coll_name.clone(),
                    index_name: name,
                    key: serde_json::to_string(&key_doc).unwrap_or_default(),
                    unique,
                    sparse,
                    expire_after_seconds: expire,
                });
            }

            // Drop up to max_per_collection indexes
            for entry in droppable.into_iter().take(params.max_per_collection) {
                match coll.drop_index(&entry.index_name).await {
                    Ok(_) => {
                        tracing::info!(
                            collection = %coll_name,
                            index = %entry.index_name,
                            "Dropped index"
                        );
                        all_undo.push(entry);
                    }
                    Err(e) => {
                        tracing::warn!(
                            collection = %coll_name,
                            index = %entry.index_name,
                            error = %e,
                            "Failed to drop index"
                        );
                    }
                }
            }
        }

        tracing::info!(dropped = all_undo.len(), "Index drop complete");

        let undo_state = serde_yaml::to_value(&all_undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("mongo.index_drop", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let entries: Vec<IndexUndoEntry> = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        for entry in &entries {
            let db = client.database(&entry.database);
            let coll = db.collection::<Document>(&entry.collection);

            // Reconstruct the key document
            let key_doc: Document = match serde_json::from_str(&entry.key) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(index = %entry.index_name, error = %e, "Failed to parse index key");
                    continue;
                }
            };

            let mut opts = mongodb::options::IndexOptions::default();
            opts.name = Some(entry.index_name.clone());
            opts.unique = Some(entry.unique);
            opts.sparse = Some(entry.sparse);
            if let Some(secs) = entry.expire_after_seconds {
                opts.expire_after = Some(std::time::Duration::from_secs(secs as u64));
            }

            let index_model = mongodb::IndexModel::builder()
                .keys(key_doc)
                .options(opts)
                .build();

            match coll.create_index(index_model).await {
                Ok(_) => {
                    tracing::info!(
                        collection = %entry.collection,
                        index = %entry.index_name,
                        "Rollback: recreated index"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        index = %entry.index_name,
                        error = %e,
                        "Rollback: failed to recreate index"
                    );
                }
            }
        }

        Ok(())
    }
}
