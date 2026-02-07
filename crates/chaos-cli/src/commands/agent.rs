use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use clap::Args;

use chaos_core::agent::Agent;
use chaos_core::config::ChaosConfig;
use chaos_core::event::TracingEventSink;
use chaos_core::experiment::ExperimentConfig;
use chaos_core::orchestrator::Orchestrator;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_db::config::{DbTargetConfig, DbType};
use chaos_k8s::agent::K8sAgent;
use chaos_k8s::config::K8sTargetConfig;
use chaos_llm::mcp::{McpClient, McpServerConfig};
use chaos_llm::planner::ChaosPlanner;
use chaos_llm::provider::LlmProviderConfig;
use chaos_llm::tool::{Tool, ToolDefinition};
use chaos_server::agent::ServerAgent;
use chaos_server::config::ServerTargetConfig;

#[derive(Debug, serde::Deserialize)]
struct PlanConfig {
    llm: LlmProviderConfig,
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default = "default_max_turns")]
    max_turns: u32,
}

fn default_max_turns() -> u32 {
    10
}

#[derive(Args)]
pub struct AgentArgs {
    /// User prompt describing what chaos to create
    pub prompt: String,
    /// Path to LLM/MCP config file
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// LLM provider: anthropic, openai, or ollama (auto-detected from API key env vars if not set)
    #[arg(long, env = "CHAOS_PROVIDER")]
    pub provider: Option<String>,
    /// Model to use
    #[arg(long, env = "CHAOS_MODEL")]
    pub model: Option<String>,
    /// API key (or set via ANTHROPIC_API_KEY / OPENAI_API_KEY env var)
    #[arg(long)]
    pub api_key: Option<String>,
    /// Dry-run: show generated config without executing
    #[arg(long)]
    pub dry_run: bool,
    /// Save the generated experiment config to a YAML file
    #[arg(long)]
    pub save: Option<PathBuf>,
    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub async fn execute(args: AgentArgs) -> anyhow::Result<()> {
    // --- Phase 1: Planning ---
    let plan_result = if let Some(config_path) = &args.config {
        let content = std::fs::read_to_string(config_path)?;
        let plan_config: PlanConfig = serde_yaml::from_str(&content)?;

        let mut planner = ChaosPlanner::new(&plan_config.llm);
        planner.set_verbose(true);
        planner.update_skills(collect_skill_definitions());
        planner.register_tool(Box::new(LiveDiscoverResourcesTool));

        if let Some(prompt) = plan_config.system_prompt {
            planner.set_system_prompt(prompt);
        }
        planner.set_max_turns(plan_config.max_turns);

        for mcp_config in plan_config.mcp_servers {
            let client = McpClient::new(mcp_config);
            planner.add_mcp_server(client).await?;
        }

        println!("Planning chaos experiments...\n");
        planner.plan(&args.prompt).await?
    } else {
        let provider_config = build_provider_config(&args)?;
        let mut planner = ChaosPlanner::new(&provider_config);
        planner.set_verbose(true);
        planner.update_skills(collect_skill_definitions());
        planner.register_tool(Box::new(LiveDiscoverResourcesTool));

        println!("Planning chaos experiments...\n");
        planner.plan(&args.prompt).await?
    };

    // --- Display plan ---
    println!("{}", plan_result.message);

    if plan_result.experiments.is_empty() {
        println!("\nNo experiments were planned.");
        return Ok(());
    }

    println!("\nPlanned experiments ({}):", plan_result.experiments.len());
    for (i, exp) in plan_result.experiments.iter().enumerate() {
        println!(
            "  {}. {} (target: {})",
            i + 1,
            exp["name"].as_str().unwrap_or("unnamed"),
            exp["target"].as_str().unwrap_or("unknown"),
        );
    }
    println!("\n(Completed in {} turns)", plan_result.turns);

    // --- Phase 2: Convert to ExperimentConfig ---
    let experiment_configs = convert_experiments(&plan_result.experiments)?;
    let chaos_config = ChaosConfig {
        experiments: experiment_configs,
    };

    let yaml_output = serde_yaml::to_string(&chaos_config)?;

    // --- Save if requested ---
    if let Some(ref save_path) = args.save {
        std::fs::write(save_path, &yaml_output)?;
        println!("\nSaved config to: {}", save_path.display());
    }

    // --- Dry-run: print and exit ---
    if args.dry_run {
        println!("\n--- Generated Configuration (dry-run) ---\n");
        println!("{yaml_output}");
        return Ok(());
    }

    // --- Confirmation ---
    println!("\n--- Generated Configuration ---\n");
    println!("{yaml_output}");

    if !args.yes && !confirm_execution() {
        println!("Aborted.");
        return Ok(());
    }

    // --- Phase 3: Execute ---
    let mut orchestrator = Orchestrator::new();
    orchestrator.add_event_sink(Arc::new(TracingEventSink));

    for experiment in &chaos_config.experiments {
        match experiment.target {
            TargetDomain::Database => {
                let agent = DbAgent::from_yaml(&experiment.target_config)?;
                orchestrator.register_agent(Box::new(agent));
            }
            TargetDomain::Kubernetes => {
                let agent = K8sAgent::from_yaml(&experiment.target_config)?;
                orchestrator.register_agent(Box::new(agent));
            }
            TargetDomain::Server => {
                let agent = ServerAgent::from_yaml(&experiment.target_config)?;
                orchestrator.register_agent(Box::new(agent));
            }
        }
    }

    for experiment in chaos_config.experiments {
        tracing::info!(name = %experiment.name, "Starting experiment");
        match orchestrator.run_experiment(experiment.clone()).await {
            Ok(report) => {
                println!("{report}");
            }
            Err(e) => {
                eprintln!("Experiment '{}' failed: {e}", experiment.name);
            }
        }
    }

    Ok(())
}

fn convert_experiments(
    json_experiments: &[serde_json::Value],
) -> anyhow::Result<Vec<ExperimentConfig>> {
    json_experiments
        .iter()
        .enumerate()
        .map(|(i, exp)| {
            let json_str = serde_json::to_string(exp)?;
            let config: ExperimentConfig = serde_yaml::from_str(&json_str).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse experiment #{} '{}': {e}",
                    i + 1,
                    exp["name"].as_str().unwrap_or("unnamed")
                )
            })?;
            Ok(config)
        })
        .collect()
}

fn confirm_execution() -> bool {
    use std::io::{self, Write};
    print!("Proceed with execution? [y/N] ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

fn detect_provider(args: &AgentArgs) -> String {
    if let Some(ref provider) = args.provider {
        return provider.clone();
    }
    if args.api_key.is_some() {
        return "anthropic".to_string();
    }
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return "anthropic".to_string();
    }
    if std::env::var("OPENAI_API_KEY").is_ok() {
        return "openai".to_string();
    }
    "ollama".to_string()
}

/// Live implementation of discover_resources that actually connects to the target.
struct LiveDiscoverResourcesTool;

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
        let target_config_json = &arguments["target_config"];

        // Convert JSON target_config to serde_yaml::Value
        let json_str = serde_json::to_string(target_config_json)?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&json_str)?;

        let mut agent: Box<dyn Agent> = match target {
            "database" | "db" => {
                Box::new(DbAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            "kubernetes" | "k8s" => {
                Box::new(K8sAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            "server" | "srv" => {
                Box::new(ServerAgent::from_yaml(&yaml_value).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
            other => anyhow::bail!("Unknown target: {other}"),
        };

        // Actually connect and discover
        agent.initialize().await.map_err(|e| anyhow::anyhow!("Failed to initialize: {e}"))?;
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
fn collect_skill_definitions() -> Vec<ToolDefinition> {
    let db_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::Postgres,
        schemas: Vec::new(),
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
        vec![&db_agent, &k8s_agent, &server_agent];

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

fn build_provider_config(args: &AgentArgs) -> anyhow::Result<LlmProviderConfig> {
    let provider = detect_provider(args);
    match provider.as_str() {
        "anthropic" => {
            let api_key = args
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Anthropic API key required: use --api-key or set ANTHROPIC_API_KEY"
                    )
                })?;
            Ok(LlmProviderConfig::Anthropic {
                api_key,
                model: args
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
                max_tokens: 4096,
            })
        }
        "openai" => {
            let api_key = args
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("OpenAI API key required: use --api-key or set OPENAI_API_KEY")
                })?;
            Ok(LlmProviderConfig::Openai {
                api_key,
                model: args
                    .model
                    .clone()
                    .unwrap_or_else(|| "gpt-4o".to_string()),
                base_url: None,
                max_tokens: 4096,
            })
        }
        "ollama" => Ok(LlmProviderConfig::Ollama {
            base_url: "http://localhost:11434".to_string(),
            model: args
                .model
                .clone()
                .unwrap_or_else(|| "llama3.1".to_string()),
            max_tokens: 4096,
        }),
        other => anyhow::bail!("Unknown provider: {other}. Use: anthropic, openai, or ollama"),
    }
}
