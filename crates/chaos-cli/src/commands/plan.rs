use clap::Args;
use std::path::PathBuf;

use chaos_llm::mcp::{McpClient, McpServerConfig};
use chaos_llm::planner::ChaosPlanner;
use chaos_llm::provider::LlmProviderConfig;

/// Configuration file for the `plan` command.
#[derive(Debug, serde::Deserialize)]
struct PlanConfig {
    /// LLM provider configuration.
    llm: LlmProviderConfig,
    /// Optional MCP servers to connect to for additional tools.
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
    /// System prompt override.
    #[serde(default)]
    system_prompt: Option<String>,
    /// Max agentic turns.
    #[serde(default = "default_max_turns")]
    max_turns: u32,
}

fn default_max_turns() -> u32 {
    10
}

#[derive(Args)]
pub struct PlanArgs {
    /// User prompt describing what chaos to create
    pub prompt: String,
    /// Path to LLM/MCP config file
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// LLM provider: anthropic, openai, or ollama
    #[arg(long, default_value = "anthropic")]
    pub provider: String,
    /// Model to use
    #[arg(long)]
    pub model: Option<String>,
    /// API key (or set via ANTHROPIC_API_KEY / OPENAI_API_KEY env var)
    #[arg(long)]
    pub api_key: Option<String>,
}

pub async fn execute(args: PlanArgs) -> anyhow::Result<()> {
    let provider_config = if let Some(config_path) = &args.config {
        let content = std::fs::read_to_string(config_path)?;
        let plan_config: PlanConfig = serde_yaml::from_str(&content)?;

        let mut planner = ChaosPlanner::new(&plan_config.llm);

        if let Some(prompt) = plan_config.system_prompt {
            planner.set_system_prompt(prompt);
        }
        planner.set_max_turns(plan_config.max_turns);

        // Connect MCP servers
        for mcp_config in plan_config.mcp_servers {
            let client = McpClient::new(mcp_config);
            planner.add_mcp_server(client).await?;
        }

        return run_planner(planner, &args.prompt).await;
    } else {
        // Build config from CLI args
        build_provider_config(&args)?
    };

    let planner = ChaosPlanner::new(&provider_config);
    run_planner(planner, &args.prompt).await
}

fn build_provider_config(args: &PlanArgs) -> anyhow::Result<LlmProviderConfig> {
    match args.provider.as_str() {
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

async fn run_planner(mut planner: ChaosPlanner, prompt: &str) -> anyhow::Result<()> {
    println!("Planning chaos experiments...\n");

    let result = planner.plan(prompt).await?;

    println!("{}", result.message);

    if !result.experiments.is_empty() {
        println!("\nPlanned experiments ({}):", result.experiments.len());
        for (i, exp) in result.experiments.iter().enumerate() {
            println!(
                "  {}. {} (target: {})",
                i + 1,
                exp["name"].as_str().unwrap_or("unnamed"),
                exp["target"].as_str().unwrap_or("unknown"),
            );
        }
    }

    println!("\n(Completed in {} turns)", result.turns);

    Ok(())
}
