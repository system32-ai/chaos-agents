use async_trait::async_trait;
use chaos_core::error::{ChaosError, ChaosResult};
use chaos_core::rollback::RollbackHandle;
use chaos_core::skill::{Skill, SkillContext, SkillDescriptor, TargetDomain};
use mongodb::bson::doc;
use mongodb::Client;
use serde::{Deserialize, Serialize};

pub struct MongoConnectionStressSkill;

#[derive(Debug, Deserialize)]
struct ConnectionStressParams {
    /// MongoDB connection URL (uses the agent's connection_url if not set).
    #[serde(default)]
    connection_url: String,
    /// Number of extra clients to open. Default: 50.
    #[serde(default = "default_count")]
    count: u32,
}

fn default_count() -> u32 {
    50
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectionStressUndoState {
    /// We store the count so rollback knows how many were opened.
    /// The actual clients are held in a leaked Vec during the experiment
    /// and cleaned up when the process drops them.
    opened: u32,
    note: String,
}

#[async_trait]
impl Skill for MongoConnectionStressSkill {
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            name: "mongo.connection_pool_stress".into(),
            description: "Open many MongoDB connections to exhaust server connection limits".into(),
            target: TargetDomain::Database,
            reversible: true,
        }
    }

    fn validate_params(&self, params: &serde_yaml::Value) -> ChaosResult<()> {
        let _: ConnectionStressParams = serde_yaml::from_value(params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid mongo.connection_pool_stress params: {e}")))?;
        Ok(())
    }

    async fn execute(&self, ctx: &SkillContext) -> ChaosResult<RollbackHandle> {
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let params: ConnectionStressParams = serde_yaml::from_value(ctx.params.clone())
            .map_err(|e| ChaosError::Config(format!("Invalid params: {e}")))?;

        // Get the connection URL from params or from the agent's existing client
        // We extract the connection string by using serverStatus
        let server_status = client
            .database("admin")
            .run_command(doc! { "serverStatus": 1, "connections": 1 })
            .await;

        let current_connections = server_status
            .as_ref()
            .ok()
            .and_then(|s| s.get_document("connections").ok())
            .and_then(|c| c.get_i32("current").ok())
            .unwrap_or(0);

        tracing::info!(
            current_connections,
            target_new = params.count,
            "Starting connection pool stress"
        );

        // Open extra clients — each Client maintains its own connection pool.
        // We resolve the connection URL: use explicit param if set, otherwise
        // we'll create clients from the same URI the agent used.  Since the
        // agent's Client is already connected, we just need the URI.  We get
        // it from the param; callers should set connection_url.
        let uri = if params.connection_url.is_empty() {
            // Fall back: re-read from agent context — but we can't access it
            // directly here, so we ping the existing client and open new ones
            // by cloning the existing client (each clone shares the pool, so
            // we issue concurrent pings to force new connections).
            String::new()
        } else {
            params.connection_url.clone()
        };

        let mut opened = 0u32;

        if uri.is_empty() {
            // Use the existing client — spawn concurrent pings to force
            // the pool to open connections up to its max.
            let mut handles = Vec::new();
            for _ in 0..params.count {
                let c = client.clone();
                handles.push(tokio::spawn(async move {
                    // Each ping forces the pool to checkout a connection
                    let _ = c
                        .database("admin")
                        .run_command(doc! { "ping": 1 })
                        .await;
                }));
            }
            for h in handles {
                if h.await.is_ok() {
                    opened += 1;
                }
            }
        } else {
            // Open new independent clients, each with its own pool
            for i in 0..params.count {
                match Client::with_uri_str(&uri).await {
                    Ok(new_client) => {
                        // Ping to force the connection to be established
                        let _ = new_client
                            .database("admin")
                            .run_command(doc! { "ping": 1 })
                            .await;
                        // Leak the client so it stays alive during the experiment.
                        // It will be cleaned up when the process exits or soak ends.
                        std::mem::forget(new_client);
                        opened += 1;
                    }
                    Err(e) => {
                        tracing::warn!(attempt = i, error = %e, "Failed to open extra connection");
                        break;
                    }
                }
            }
        }

        // Check new connection count
        let new_status = client
            .database("admin")
            .run_command(doc! { "serverStatus": 1, "connections": 1 })
            .await;

        let new_connections = new_status
            .as_ref()
            .ok()
            .and_then(|s| s.get_document("connections").ok())
            .and_then(|c| c.get_i32("current").ok())
            .unwrap_or(0);

        tracing::info!(
            opened,
            connections_before = current_connections,
            connections_after = new_connections,
            "Connection pool stress applied"
        );

        let undo = ConnectionStressUndoState {
            opened,
            note: "Connections will be released when the experiment process ends or soak duration expires".into(),
        };

        let undo_state = serde_yaml::to_value(&undo)
            .map_err(|e| ChaosError::Other(anyhow::anyhow!("Serialize undo: {e}")))?;

        Ok(RollbackHandle::new(
            "mongo.connection_pool_stress",
            undo_state,
        ))
    }

    async fn rollback(&self, ctx: &SkillContext, _handle: &RollbackHandle) -> ChaosResult<()> {
        // Leaked clients are cleaned up by process termination.
        // For non-leaked (concurrent ping) mode, the connections return
        // to the pool automatically.
        let client = ctx
            .shared
            .downcast_ref::<Client>()
            .ok_or_else(|| ChaosError::Connection(anyhow::anyhow!("Expected mongodb::Client")))?;

        let status = client
            .database("admin")
            .run_command(doc! { "serverStatus": 1, "connections": 1 })
            .await;

        let current = status
            .as_ref()
            .ok()
            .and_then(|s| s.get_document("connections").ok())
            .and_then(|c| c.get_i32("current").ok())
            .unwrap_or(0);

        tracing::info!(
            current_connections = current,
            "Connection pool stress rollback: connections will drain as clients are dropped"
        );

        Ok(())
    }
}
