use crate::mcp::McpClient;
use crate::provider::{
    create_provider, ChatMessage, FinishReason, LlmProvider, LlmProviderConfig, Role,
};
use crate::tool::{
    DiscoverResourcesTool, ListSkillsTool, RunExperimentTool, ToolDefinition, ToolRegistry,
};

/// The LLM-driven chaos planner.
///
/// This component uses an LLM to decide which chaos experiments to run based on
/// discovered infrastructure and available skills. It provides the LLM with tools
/// to list skills, discover resources, and run experiments.
pub struct ChaosPlanner {
    provider: Box<dyn LlmProvider>,
    tool_registry: ToolRegistry,
    mcp_clients: Vec<McpClient>,
    system_prompt: String,
    messages: Vec<ChatMessage>,
    max_turns: u32,
    verbose: bool,
}

impl ChaosPlanner {
    pub fn new(provider_config: &LlmProviderConfig) -> Self {
        let provider = create_provider(provider_config);
        let mut tool_registry = ToolRegistry::new();

        // Register built-in tools
        tool_registry.register(Box::new(ListSkillsTool {
            skills: Vec::new(), // populated during init
        }));
        tool_registry.register(Box::new(RunExperimentTool));
        tool_registry.register(Box::new(DiscoverResourcesTool));

        Self {
            provider,
            tool_registry,
            mcp_clients: Vec::new(),
            system_prompt: default_system_prompt(),
            messages: Vec::new(),
            max_turns: 10,
            verbose: false,
        }
    }

    /// Add an MCP server to provide additional tools.
    pub async fn add_mcp_server(&mut self, mut client: McpClient) -> anyhow::Result<()> {
        client.initialize().await?;
        client.register_tools(&mut self.tool_registry);
        self.mcp_clients.push(client);
        Ok(())
    }

    /// Register a custom tool.
    pub fn register_tool(&mut self, tool: Box<dyn crate::tool::Tool>) {
        self.tool_registry.register(tool);
    }

    /// Set the system prompt.
    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    /// Set max agentic turns.
    pub fn set_max_turns(&mut self, turns: u32) {
        self.max_turns = turns;
    }

    /// Enable verbose output (prints intermediate LLM messages and tool calls to stderr).
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// Update the skills list (call after agents are initialized).
    pub fn update_skills(&mut self, skills: Vec<ToolDefinition>) {
        self.tool_registry.register(Box::new(ListSkillsTool { skills }));
    }

    /// Run the planner with a user prompt.
    /// Returns the final assistant message and a list of experiment configs it wants to run.
    pub async fn plan(&mut self, user_prompt: &str) -> anyhow::Result<PlanResult> {
        self.messages.clear();

        // Add system message
        self.messages.push(ChatMessage {
            role: Role::System,
            content: self.system_prompt.clone(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });

        // Add user message
        self.messages.push(ChatMessage {
            role: Role::User,
            content: user_prompt.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });

        let tool_defs = self.tool_registry.definitions();
        let mut experiments = Vec::new();

        // Track target configs from discover_resources calls so we can inject them
        // into run_experiment calls if the LLM omits them.
        let mut discovered_targets: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        for turn in 0..self.max_turns {
            tracing::info!(turn, "LLM planner turn");
            if self.verbose {
                eprintln!("[turn {}/{}] Thinking...", turn + 1, self.max_turns);
            }

            let response = self.provider.chat(&self.messages, &tool_defs).await?;

            if let Some(usage) = &response.usage {
                tracing::debug!(
                    input = usage.input_tokens,
                    output = usage.output_tokens,
                    "Token usage"
                );
            }

            // Add assistant response to history
            self.messages.push(response.message.clone());

            // Print intermediate LLM messages so the user can follow along
            if self.verbose && !response.message.content.is_empty() {
                eprintln!("[assistant] {}", response.message.content);
            }

            match response.finish_reason {
                FinishReason::Stop => {
                    tracing::info!("LLM planner finished");
                    return Ok(PlanResult {
                        message: response.message.content,
                        experiments,
                        turns: turn + 1,
                    });
                }
                FinishReason::ToolUse => {
                    // Execute each tool call
                    for tool_call in &response.message.tool_calls {
                        tracing::info!(
                            tool = %tool_call.name,
                            "Executing tool call"
                        );
                        if self.verbose {
                            eprintln!("[tool] {}()", tool_call.name);
                        }

                        let mut result = self
                            .tool_registry
                            .execute(&tool_call.name, tool_call.arguments.clone())
                            .await;
                        result.tool_call_id = tool_call.id.clone();

                        // Capture target configs from discover_resources calls
                        if tool_call.name == "discover_resources" {
                            if let (Some(target), Some(config)) = (
                                tool_call.arguments["target"].as_str(),
                                tool_call.arguments.get("target_config"),
                            ) {
                                discovered_targets
                                    .insert(target.to_string(), config.clone());
                            }
                        }

                        // Intercept run_experiment calls to capture experiment configs
                        if tool_call.name == "run_experiment" {
                            let mut exp_args = tool_call.arguments.clone();

                            // Auto-inject target_config if missing
                            let needs_inject = exp_args.get("target_config").is_none()
                                || exp_args["target_config"].is_null();
                            if needs_inject {
                                let target_key = exp_args["target"]
                                    .as_str()
                                    .map(|s| s.to_string());
                                if let Some(target) = target_key {
                                    if let Some(config) =
                                        discovered_targets.get(&target)
                                    {
                                        exp_args["target_config"] = config.clone();
                                        tracing::info!(
                                            target = %target,
                                            "Auto-injected target_config from prior discovery"
                                        );
                                    }
                                }
                            }

                            experiments.push(exp_args);
                            if self.verbose {
                                eprintln!(
                                    "[experiment] Planned: {}",
                                    tool_call.arguments["name"]
                                        .as_str()
                                        .unwrap_or("unnamed")
                                );
                            }
                        }

                        // Add tool result to conversation
                        self.messages.push(ChatMessage {
                            role: Role::Tool,
                            content: result.content,
                            tool_calls: Vec::new(),
                            tool_call_id: Some(result.tool_call_id),
                        });
                    }
                }
                FinishReason::MaxTokens => {
                    tracing::warn!("LLM hit max tokens, stopping");
                    return Ok(PlanResult {
                        message: response.message.content,
                        experiments,
                        turns: turn + 1,
                    });
                }
                FinishReason::Other(reason) => {
                    tracing::warn!(reason = %reason, "Unexpected finish reason");
                    return Ok(PlanResult {
                        message: response.message.content,
                        experiments,
                        turns: turn + 1,
                    });
                }
            }
        }

        Ok(PlanResult {
            message: "Max turns reached".to_string(),
            experiments,
            turns: self.max_turns,
        })
    }
}

/// Result of the LLM planner.
#[derive(Debug)]
pub struct PlanResult {
    /// Final message from the LLM.
    pub message: String,
    /// Experiment configs the LLM wants to execute (from run_experiment tool calls).
    pub experiments: Vec<serde_json::Value>,
    /// Number of turns used.
    pub turns: u32,
}

fn default_system_prompt() -> String {
    r#"You are a chaos engineering agent. Your job is to plan and execute controlled chaos experiments against infrastructure to test resilience.

You have access to tools to:
1. `list_skills` - List available chaos skills for databases, Kubernetes, and servers
2. `discover_resources` - Discover resources on a target (tables, pods, services)
3. `run_experiment` - Execute a chaos experiment

Your workflow — you MUST complete ALL steps without stopping to ask for confirmation:
1. First, understand what infrastructure the user wants to test
2. Use `list_skills` to see what chaos actions are available
3. Use `discover_resources` to understand the target environment
4. Plan appropriate chaos experiments based on the discovered resources
5. Use `run_experiment` to execute the chaos plan
6. After calling `run_experiment`, provide a brief summary of what was planned

CRITICAL: You are running in a non-interactive pipeline. The user has already approved execution by running this command. Do NOT ask for confirmation, feedback, or permission. Do NOT stop to explain what you will do — just do it. You MUST call `run_experiment` at least once before finishing. If discovery returns resources, proceed to plan and execute experiments immediately.

Important rules:
- Start with less destructive experiments and escalate gradually
- All experiments have automatic rollback — be mindful of duration
- When calling `run_experiment`, you MUST include `target_config` with the same connection info you used for `discover_resources`
- For servers, target relevant services based on discovery results
- Never target system-critical services (sshd, systemd, etc.)
- Keep experiment durations reasonable (1m-5m for testing)
- If discovery fails or returns no resources, still attempt a reasonable experiment based on available information"#
        .to_string()
}
