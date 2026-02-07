use async_trait::async_trait;

use chaos_core::agent::Agent;
use chaos_core::experiment::ExperimentConfig;
use chaos_core::orchestrator::Orchestrator;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_db::config::{DbTargetConfig, DbType};
use chaos_db::mongo_agent::MongoAgent;
use chaos_db::mongo_config::MongoTargetConfig;
use chaos_k8s::agent::K8sAgent;
use chaos_k8s::config::K8sTargetConfig;
use chaos_llm::provider::LlmProviderConfig;
use chaos_llm::tool::{Tool, ToolDefinition};
use chaos_server::agent::ServerAgent;
use chaos_server::config::ServerTargetConfig;

/// Live implementation of discover_resources that actually connects to the target.
pub struct LiveDiscoverResourcesTool;

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
                    "target_config": { "type": "object", "description": "Target-specific configuration (e.g. {\"connection_url\": \"postgres://...\", \"db_type\": \"postgres\"} for database)" }
                }
            }),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        let target = arguments["target"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'target' field"))?;
        let mut target_config_json = arguments["target_config"].clone();

        // Auto-detect db_type from connection_url if missing
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

        // Convert JSON target_config to serde_yaml::Value
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
                    Box::new(
                        MongoAgent::from_yaml(&yaml_value)
                            .map_err(|e| anyhow::anyhow!("{e}"))?,
                    )
                } else {
                    Box::new(
                        DbAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?,
                    )
                }
            }
            "kubernetes" | "k8s" => {
                Box::new(K8sAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            "server" | "srv" => {
                Box::new(
                    ServerAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?,
                )
            }
            other => anyhow::bail!("Unknown target: {other}"),
        };

        // Actually connect and discover
        agent
            .initialize()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize: {e}"))?;
        let resources = agent
            .discover()
            .await
            .map_err(|e| anyhow::anyhow!("Discovery failed: {e}"))?;

        // Build summary
        let mut by_type: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for r in &resources {
            by_type
                .entry(r.resource_type().to_string())
                .or_default()
                .push(r.name().to_string());
        }

        // Print stats to stderr for the user to see during planning
        eprintln!("\n  Discovery results for {target}:");
        eprintln!("  {:<15} {}", "TYPE", "COUNT");
        eprintln!("  {}", "-".repeat(30));
        for (rtype, names) in &by_type {
            eprintln!("  {:<15} {}", rtype, names.len());
        }
        eprintln!("  Total: {} resources\n", resources.len());

        // Build detailed JSON for the LLM
        let resource_list: Vec<serde_json::Value> = resources
            .iter()
            .map(|r| {
                serde_json::json!({
                    "type": r.resource_type(),
                    "name": r.name(),
                })
            })
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

/// Collect all available skill descriptors as ToolDefinitions for the LLM planner.
pub fn collect_skill_definitions() -> Vec<ToolDefinition> {
    let db_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::Postgres,
        schemas: Vec::new(),
    });
    let crdb_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::CockroachDb,
        schemas: Vec::new(),
    });
    let ysql_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::YugabyteDb,
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

    let agents: Vec<&dyn chaos_core::agent::Agent> = vec![
        &db_agent,
        &crdb_agent,
        &ysql_agent,
        &mongo_agent,
        &k8s_agent,
        &server_agent,
    ];

    let mut seen = std::collections::HashSet::new();
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
        .filter(|td| seen.insert(td.name.clone()))
        .collect()
}

/// Convert JSON experiment configs from the LLM planner into ExperimentConfig structs.
pub fn convert_experiments(
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
                    eprintln!("[fallback] Built target_config from connection URL in prompt");
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

/// Try to extract a target_config from connection URLs in the user prompt,
/// or from environment/defaults for kubernetes.
pub fn extract_target_config_from_prompt(
    prompt: &str,
    target: Option<&str>,
) -> Option<serde_json::Value> {
    // Look for database connection URLs in the prompt
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

    // For kubernetes, use KUBECONFIG env var or default ~/.kube/config
    if matches!(target, Some("kubernetes" | "k8s")) {
        let kubeconfig = std::env::var("KUBECONFIG").ok().or_else(|| {
            let home = std::env::var("HOME").ok()?;
            let default_path = format!("{home}/.kube/config");
            if std::path::Path::new(&default_path).exists() {
                Some(default_path)
            } else {
                None
            }
        });

        let namespace =
            extract_namespace_from_prompt(prompt).unwrap_or_else(|| "default".to_string());

        let mut config = serde_json::json!({ "namespace": namespace });
        if let Some(path) = kubeconfig {
            config["kubeconfig"] = serde_json::Value::String(path);
        }
        return Some(config);
    }

    None
}

/// Try to extract a kubernetes namespace from the prompt.
pub fn extract_namespace_from_prompt(prompt: &str) -> Option<String> {
    let lower = prompt.to_lowercase();
    for kw in &["namespace ", "ns "] {
        if let Some(pos) = lower.find(kw) {
            let rest = &prompt[pos + kw.len()..];
            let ns = rest.split_whitespace().next()?;
            let ns = ns.trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
            if !ns.is_empty() {
                return Some(ns.to_string());
            }
        }
    }
    None
}

/// Register the appropriate agent on the orchestrator based on experiment config.
pub fn register_agent_for_experiment(
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

/// Build a provider config from provider name, api key, model, and optional base URL.
pub fn build_provider_config_from_parts(
    provider: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    base_url: Option<&str>,
) -> anyhow::Result<LlmProviderConfig> {
    match provider {
        "anthropic" => {
            let api_key = api_key
                .map(|s| s.to_string())
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Anthropic API key required: use --api-key or set ANTHROPIC_API_KEY"
                    )
                })?;
            Ok(LlmProviderConfig::Anthropic {
                api_key,
                model: model
                    .unwrap_or("claude-sonnet-4-5-20250929")
                    .to_string(),
                max_tokens: 4096,
            })
        }
        "openai" => {
            let api_key = api_key
                .map(|s| s.to_string())
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("OpenAI API key required: use --api-key or set OPENAI_API_KEY")
                })?;
            Ok(LlmProviderConfig::Openai {
                api_key,
                model: model.unwrap_or("gpt-4o").to_string(),
                base_url: base_url.map(|s| s.to_string()),
                max_tokens: 4096,
            })
        }
        "ollama" => Ok(LlmProviderConfig::Ollama {
            base_url: base_url
                .unwrap_or("http://localhost:11434")
                .to_string(),
            model: model.unwrap_or("llama3.1").to_string(),
            max_tokens: 4096,
        }),
        other => anyhow::bail!("Unknown provider: {other}. Use: anthropic, openai, or ollama"),
    }
}
