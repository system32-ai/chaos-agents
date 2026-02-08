use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;

use chaos_core::config::ChaosConfig;
use chaos_core::event::TracingEventSink;
use chaos_core::orchestrator::Orchestrator;
use chaos_llm::mcp::{McpClient, McpServerConfig};
use chaos_llm::planner::ChaosPlanner;
use chaos_llm::provider::LlmProviderConfig;

use crate::execution::{
    build_provider_config_from_parts, collect_skill_definitions, convert_experiments,
    register_agent_for_experiment, LiveDiscoverResourcesTool,
};

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
    /// Max number of LLM planning turns (default: 10)
    #[arg(long)]
    pub max_turns: Option<u32>,
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
        planner.register_tool(Box::new(LiveDiscoverResourcesTool { user_prompt: args.prompt.clone() }));

        if let Some(prompt) = plan_config.system_prompt {
            planner.set_system_prompt(prompt);
        }
        planner.set_max_turns(args.max_turns.unwrap_or(plan_config.max_turns));

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
        planner.register_tool(Box::new(LiveDiscoverResourcesTool { user_prompt: args.prompt.clone() }));
        if let Some(max_turns) = args.max_turns {
            planner.set_max_turns(max_turns);
        }

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
    let experiment_configs = convert_experiments(&plan_result.experiments, &args.prompt)?;
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

    // Set up Ctrl+C handler to cancel experiments gracefully (rollback still runs)
    let cancel_flag = orchestrator.cancel_flag();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nReceived Ctrl+C, cancelling experiment (rollback will still run)...");
            cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });

    for experiment in &chaos_config.experiments {
        register_agent_for_experiment(&mut orchestrator, experiment)?;
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

fn build_provider_config(args: &AgentArgs) -> anyhow::Result<LlmProviderConfig> {
    let provider = detect_provider(args);
    build_provider_config_from_parts(
        &provider,
        args.api_key.as_deref(),
        args.model.as_deref(),
        None,
    )
}
