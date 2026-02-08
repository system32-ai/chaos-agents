use std::sync::Arc;

use async_trait::async_trait;

use chaos_core::agent::Agent;
use chaos_core::event::{EventSink, ExperimentEvent};
use chaos_core::experiment::ExperimentConfig;
use chaos_core::orchestrator::Orchestrator;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_db::config::{DbTargetConfig, DbType};
use chaos_db::mongo_agent::MongoAgent;
use chaos_db::mongo_config::MongoTargetConfig;
use chaos_k8s::agent::K8sAgent;
use chaos_k8s::config::K8sTargetConfig;
use chaos_llm::planner::{ChaosPlanner, PlannerEvent};
use chaos_llm::tool::{Tool, ToolDefinition};
use chaos_server::agent::ServerAgent;
use chaos_server::config::ServerTargetConfig;

use crate::wizard::WizardOutput;

/// Spawn the planner + orchestrator in a background tokio task.
/// Returns receivers for planner events and experiment events, plus a JoinHandle for cancellation.
pub fn spawn_execution(
    output: WizardOutput,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<PlannerEvent>,
    tokio::sync::mpsc::UnboundedReceiver<ExperimentEvent>,
    tokio::task::JoinHandle<()>,
) {
    let mut planner = ChaosPlanner::new(&output.provider_config);
    let planner_rx = planner.set_event_channel();
    planner.set_verbose(false);
    planner.set_max_turns(output.max_turns);
    planner.update_skills(collect_skill_definitions());
    let prompt = output.prompt.clone();
    planner.register_tool(Box::new(LiveDiscoverResourcesTool {
        user_prompt: prompt.clone(),
    }));

    // Create experiment event channel directly so we can clone the sender for error reporting
    let (exp_tx, experiment_rx) = tokio::sync::mpsc::unbounded_channel::<ExperimentEvent>();
    let error_tx = exp_tx.clone();
    let duration = output.duration.clone();

    let handle = tokio::spawn(async move {
        // Phase 1: Plan
        let enriched_prompt = format!(
            "{}\n\nExperiment duration: {}",
            prompt,
            duration,
        );

        let plan_result = match planner.plan(&enriched_prompt).await {
            Ok(r) => r,
            Err(e) => {
                let _ = error_tx.send(ExperimentEvent::Failed {
                    experiment_id: uuid::Uuid::new_v4(),
                    error: format!("Planning failed: {e}"),
                });
                return;
            }
        };

        if plan_result.experiments.is_empty() {
            let _ = error_tx.send(ExperimentEvent::Failed {
                experiment_id: uuid::Uuid::new_v4(),
                error: "No experiments were planned".into(),
            });
            return;
        }

        // Phase 2: Convert experiments
        let experiment_configs = match convert_experiments(&plan_result.experiments, &prompt) {
            Ok(c) => c,
            Err(e) => {
                let _ = error_tx.send(ExperimentEvent::Failed {
                    experiment_id: uuid::Uuid::new_v4(),
                    error: format!("Experiment conversion failed: {e}"),
                });
                return;
            }
        };

        // Phase 3: Execute
        let mut orchestrator = Orchestrator::new();
        orchestrator.add_event_sink(Arc::new(SenderEventSink(exp_tx)));

        for experiment in &experiment_configs {
            if let Err(e) = register_agent_for_experiment(&mut orchestrator, experiment) {
                let _ = error_tx.send(ExperimentEvent::Failed {
                    experiment_id: uuid::Uuid::new_v4(),
                    error: format!("Failed to register agent: {e}"),
                });
                return;
            }
        }

        for experiment in experiment_configs {
            let _ = orchestrator.run_experiment(experiment).await;
        }
    });

    (planner_rx, experiment_rx, handle)
}

/// Thin EventSink wrapper around an UnboundedSender so we can clone the sender for error reporting.
struct SenderEventSink(tokio::sync::mpsc::UnboundedSender<ExperimentEvent>);

#[async_trait]
impl EventSink for SenderEventSink {
    async fn emit(&self, event: ExperimentEvent) {
        let _ = self.0.send(event);
    }
}

// --- Duplicated from chaos-cli/src/execution.rs to avoid circular dependency ---

struct LiveDiscoverResourcesTool {
    user_prompt: String,
}

#[async_trait]
impl Tool for LiveDiscoverResourcesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "discover_resources".into(),
            description: "Discover resources (tables, pods, services) on a chaos target. Returns actual discovered resources.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["target", "target_config"],
                "properties": {
                    "target": { "type": "string", "enum": ["database", "kubernetes", "server"] },
                    "target_config": {
                        "type": "object",
                        "description": "Target connection config. For database: {\"connection_url\": \"postgres://user:pass@host:5432/db\", \"db_type\": \"postgres\"} (db_type values: postgres, mysql, cockroach_db, yugabyte_db, mongo_d_b). For kubernetes: {\"namespace\": \"default\"}. For server: {\"hosts\": [{\"host\": \"1.2.3.4\", \"port\": 22, \"username\": \"user\", \"auth\": {\"type\": \"key\", \"private_key_path\": \"~/.ssh/id_ed25519\"}}]}"
                    }
                }
            }),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        let target = arguments["target"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'target' field"))?;
        let mut target_config_json = arguments["target_config"].clone();

        // Fallback: extract connection_url from user prompt if LLM omitted it
        if matches!(target, "database" | "db") {
            if target_config_json.get("connection_url").map_or(true, |v| v.is_null() || v.as_str().map_or(true, |s| s.is_empty())) {
                if let Some(config) = extract_target_config_from_prompt(&self.user_prompt, Some(target)) {
                    // Merge: prompt-extracted values fill in missing fields
                    if let Some(obj) = config.as_object() {
                        for (k, v) in obj {
                            if target_config_json.get(k).map_or(true, |existing| existing.is_null()) {
                                target_config_json[k.clone()] = v.clone();
                            }
                        }
                    }
                }
            }
        }

        // Auto-detect db_type from connection_url if still missing
        if matches!(target, "database" | "db") {
            if target_config_json.get("db_type").map_or(true, |v| v.is_null()) {
                if let Some(url) = target_config_json.get("connection_url").and_then(|v| v.as_str()) {
                    let db_type = if url.starts_with("mongodb://") || url.starts_with("mongodb+srv://") {
                        "mongo_d_b"
                    } else if url.starts_with("mysql://") {
                        "mysql"
                    } else {
                        "postgres"
                    };
                    target_config_json["db_type"] = serde_json::Value::String(db_type.to_string());
                }
            }
        }

        // Fallback: extract k8s config from prompt if missing
        if matches!(target, "kubernetes" | "k8s") {
            if target_config_json.get("namespace").map_or(true, |v| v.is_null()) {
                if let Some(config) = extract_target_config_from_prompt(&self.user_prompt, Some(target)) {
                    if let Some(obj) = config.as_object() {
                        for (k, v) in obj {
                            if target_config_json.get(k).map_or(true, |existing| existing.is_null()) {
                                target_config_json[k.clone()] = v.clone();
                            }
                        }
                    }
                }
            }
        }

        let json_str = serde_json::to_string(&target_config_json)?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&json_str)?;

        let mut agent: Box<dyn Agent> = match target {
            "database" | "db" => {
                let is_mongo = target_config_json
                    .get("db_type")
                    .and_then(|v| v.as_str())
                    .map_or(false, |t| t == "mongo_d_b" || t == "mongodb" || t == "mongo")
                    || target_config_json
                        .get("connection_url")
                        .and_then(|v| v.as_str())
                        .map_or(false, |u| {
                            u.starts_with("mongodb://") || u.starts_with("mongodb+srv://")
                        });
                if is_mongo {
                    Box::new(MongoAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
                } else {
                    Box::new(DbAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
                }
            }
            "kubernetes" | "k8s" => {
                Box::new(K8sAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            "server" | "srv" => {
                Box::new(ServerAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            other => anyhow::bail!("Unknown target: {other}"),
        };

        agent.initialize().await.map_err(|e| anyhow::anyhow!("Failed to initialize: {e}"))?;
        let resources = agent.discover().await.map_err(|e| anyhow::anyhow!("Discovery failed: {e}"))?;

        let mut by_type: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for r in &resources {
            by_type
                .entry(r.resource_type().to_string())
                .or_default()
                .push(r.name().to_string());
        }

        let resource_list: Vec<serde_json::Value> = resources
            .iter()
            .map(|r| serde_json::json!({"type": r.resource_type(), "name": r.name()}))
            .collect();

        let result = serde_json::json!({
            "target": target,
            "total_resources": resources.len(),
            "resources_by_type": by_type,
            "resources": resource_list,
        });

        Ok(serde_json::to_string_pretty(&result)?)
    }
}

fn collect_skill_definitions() -> Vec<ToolDefinition> {
    let db_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::Postgres,
        schemas: Vec::new(),
    });
    let mongo_agent = MongoAgent::new(MongoTargetConfig {
        connection_url: String::new(),
        databases: Vec::new(),
    });
    let k8s_agent = K8sAgent::new(K8sTargetConfig {
        kubeconfig: None,
        namespace: "default".into(),
        label_selector: None,
    });
    let server_agent = ServerAgent::new(ServerTargetConfig {
        hosts: Vec::new(),
        discovery: Default::default(),
    });

    let agents: Vec<&dyn chaos_core::agent::Agent> =
        vec![&db_agent, &mongo_agent, &k8s_agent, &server_agent];

    agents
        .iter()
        .flat_map(|agent| {
            agent.skills().into_iter().map(|skill| {
                let desc = skill.descriptor();
                ToolDefinition {
                    name: desc.name.clone(),
                    description: format!(
                        "[{}] {} (reversible: {})",
                        desc.target, desc.description, desc.reversible
                    ),
                    parameters: serde_json::json!({}),
                }
            })
        })
        .collect()
}

fn convert_experiments(
    json_experiments: &[serde_json::Value],
    user_prompt: &str,
) -> anyhow::Result<Vec<ExperimentConfig>> {
    json_experiments
        .iter()
        .enumerate()
        .map(|(i, exp)| {
            let mut exp = exp.clone();

            // If target_config is missing, try to build one from the user prompt
            let has_target_config = exp
                .get("target_config")
                .map_or(false, |v| !v.is_null() && v.is_object());
            if !has_target_config {
                if let Some(config) =
                    extract_target_config_from_prompt(user_prompt, exp["target"].as_str())
                {
                    exp["target_config"] = config;
                }
            }

            let json_str = serde_json::to_string(&exp)?;
            let config: ExperimentConfig = serde_yaml::from_str(&json_str).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse experiment #{} '{}': {e}\nGenerated JSON: {}",
                    i + 1,
                    exp["name"].as_str().unwrap_or("unnamed"),
                    serde_json::to_string_pretty(&exp).unwrap_or_default()
                )
            })?;
            Ok(config)
        })
        .collect()
}

fn register_agent_for_experiment(
    orchestrator: &mut Orchestrator,
    experiment: &ExperimentConfig,
) -> anyhow::Result<()> {
    match experiment.target {
        TargetDomain::Database => {
            let is_mongo = experiment
                .target_config
                .get("db_type")
                .and_then(|v| v.as_str())
                .map_or(false, |t| t == "mongo_d_b" || t == "mongodb" || t == "mongo");
            if is_mongo {
                let agent = MongoAgent::from_yaml(&experiment.target_config)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                orchestrator.register_agent(Box::new(agent));
            } else {
                let agent = DbAgent::from_yaml(&experiment.target_config)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                orchestrator.register_agent(Box::new(agent));
            }
        }
        TargetDomain::Kubernetes => {
            let agent = K8sAgent::from_yaml(&experiment.target_config)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            orchestrator.register_agent(Box::new(agent));
        }
        TargetDomain::Server => {
            let agent = ServerAgent::from_yaml(&experiment.target_config)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            orchestrator.register_agent(Box::new(agent));
        }
    }
    Ok(())
}

/// Extract target_config from connection URLs found in the user prompt.
fn extract_target_config_from_prompt(
    prompt: &str,
    target: Option<&str>,
) -> Option<serde_json::Value> {
    for word in prompt.split_whitespace() {
        let word = word.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == ')');
        if word.starts_with("postgres://") || word.starts_with("postgresql://") {
            let lower = prompt.to_lowercase();
            let db_type = if lower.contains("cockroach") || lower.contains("crdb") {
                "cockroach_db"
            } else if lower.contains("yugabyte") || lower.contains("ysql") {
                "yugabyte_db"
            } else {
                "postgres"
            };
            return Some(serde_json::json!({
                "connection_url": word,
                "db_type": db_type
            }));
        }
        if word.starts_with("mysql://") {
            return Some(serde_json::json!({
                "connection_url": word,
                "db_type": "mysql"
            }));
        }
        if word.starts_with("mongodb://") || word.starts_with("mongodb+srv://") {
            return Some(serde_json::json!({
                "connection_url": word,
                "db_type": "mongo_d_b"
            }));
        }
    }

    if matches!(target, Some("kubernetes" | "k8s")) {
        let namespace = prompt
            .to_lowercase()
            .find("namespace ")
            .and_then(|pos| {
                prompt[pos + 10..]
                    .split_whitespace()
                    .next()
                    .map(|s| s.trim_matches(|c: char| c == '"' || c == '\'' || c == ',').to_string())
            })
            .unwrap_or_else(|| "default".to_string());

        let mut config = serde_json::json!({ "namespace": namespace });
        if let Ok(kubeconfig) = std::env::var("KUBECONFIG") {
            config["kubeconfig"] = serde_json::Value::String(kubeconfig);
        }
        return Some(config);
    }

    None
}
