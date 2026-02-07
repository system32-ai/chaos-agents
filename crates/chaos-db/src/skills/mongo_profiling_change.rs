use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use mongodb::bson::doc;
use mongodb::Client;
use serde::{Deserialize, Serialize};

pub struct MongoProfilingChangeSkill;

#[derive(Debug, Deserialize)]
struct ProfilingParams {
    #[serde(default = "default_db")]
    database: String,
    /// Profiling level: 0 = off, 1 = slow ops only, 2 = all ops.
    /// Level 2 adds significant overhead and is the default chaos action.
    #[serde(default = "default_level")]
    level: i32,
    /// Slow operation threshold in milliseconds (only used when level=1).
    #[serde(default = "default_slow_ms")]
    slow_ms: i32,
}

fn default_db() -> String {
    "test".to_string()
}

fn default_level() -> i32 {
    2
}

fn default_slow_ms() -> i32 {
    100
}

#[derive(Debug, Serialize, Deserialize)]
struct ProfilingUndoState {
    database: String,
    original_level: i32,
    original_slow_ms: i32,
}

#[async_trait]
impl Skill for MongoProfilingChangeSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.profiling_change".into(),
            description: "Change MongoDB profiling level to add overhead (level 2 logs all operations)".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let p: ProfilingParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.profiling_change params: {e}")))?;
        if !(0..=2).contains(&p.level) {
            return Err(ChaosError::Config("profiling level must be 0, 1, or 2".into()));
        }
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: ProfilingParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        let db = client.database(&params.database);

        // Get current profiling level
        let profile_result = db
            .run_command(doc! { "profile": -1 })
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!("Failed to get profiling level: {e}"))
            })?;

        let original_level = profile_result.get_i32("was").unwrap_or(0);
        let original_slow_ms = profile_result.get_i32("slowms").unwrap_or(100);

        // Set new profiling level
        let mut cmd = doc! { "profile": params.level };
        if params.level == 1 {
            cmd.insert("slowms", params.slow_ms);
        }

        db.run_command(cmd).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!("Failed to set profiling level: {e}"))
        })?;

        tracing::info!(
            database = %params.database,
            old_level = original_level,
            new_level = params.level,
            "Profiling level changed"
        );

        let undo = ProfilingUndoState {
            database: params.database,
            original_level,
            original_slow_ms,
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new("mongo.profiling_change", undo_state))
    }

    async fn rollback(&self, ctx: &SkillContext, handle: &RollbackHandle) -> ChaosResult<()> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let undo: ProfilingUndoState = serde_yaml::from_value(handle.undo_state.clone())
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Parse undo: {e}")))?;

        let db = client.database(&undo.database);

        let mut cmd = doc! { "profile": undo.original_level };
        if undo.original_level == 1 {
            cmd.insert("slowms", undo.original_slow_ms);
        }

        match db.run_command(cmd).await {
            Ok(_) => {
                tracing::info!(
                    database = %undo.database,
                    level = undo.original_level,
                    "Rollback: profiling level restored"
                );
            }
            Err(e) => {
                tracing::error!(
                    database = %undo.database,
                    error = %e,
                    "Rollback: failed to restore profiling level"
                );
            }
        }

        Ok(())
    }
}
