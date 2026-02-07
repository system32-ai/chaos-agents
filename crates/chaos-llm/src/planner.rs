use crate::mcp::McpClient;
use crate::provider::{
    create_provider, ChatMessage, FinishReason, LlmProvider, LlmProviderConfig, Role,
};
use crate::tool::{
    DiscoverResourcesTool, ListSkillsTool, RunExperimentTool, ToolDefinition, ToolRegistry,
};

/// Events emitted during LLM planning for UI consumption.
#[derive(Debug, Clone)]
pub enum PlannerEvent {
    TurnStarted { turn: u32, max_turns: u32 },
    AssistantMessage { content: String },
    ToolCallStarted { name: String, arguments: serde_json::Value },
    ToolCallCompleted { name: String, result: String, is_error: bool },
    ExperimentPlanned { name: String, target: String },
    DiscoveryResult { target: String, resource_count: usize },
    PlanningComplete { turns: u32, experiment_count: usize },
    TokenUsage { input_tokens: u32, output_tokens: u32 },
}

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
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<PlannerEvent>>,
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
            event_tx: None,
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

    /// Set up an event channel for TUI consumption.
    /// Returns the receiver end of the channel.
    pub fn set_event_channel(&mut self) -> tokio::sync::mpsc::UnboundedReceiver<PlannerEvent> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.event_tx = Some(tx);
        rx
    }

    fn emit_event(&self, event: PlannerEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
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
            self.emit_event(PlannerEvent::TurnStarted {
                turn: turn + 1,
                max_turns: self.max_turns,
            });
            if self.verbose && self.event_tx.is_none() {
                eprintln!("[turn {}/{}] Thinking...", turn + 1, self.max_turns);
            }

            let response = self.provider.chat(&self.messages, &tool_defs).await?;

            if let Some(usage) = &response.usage {
                tracing::debug!(
                    input = usage.input_tokens,
                    output = usage.output_tokens,
                    "Token usage"
                );
                self.emit_event(PlannerEvent::TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                });
            }

            // Add assistant response to history
            self.messages.push(response.message.clone());

            // Emit assistant message
            if !response.message.content.is_empty() {
                self.emit_event(PlannerEvent::AssistantMessage {
                    content: response.message.content.clone(),
                });
                if self.verbose && self.event_tx.is_none() {
                    eprintln!("[assistant] {}", response.message.content);
                }
            }

            match response.finish_reason {
                FinishReason::Stop => {
                    tracing::info!("LLM planner finished");
                    self.emit_event(PlannerEvent::PlanningComplete {
                        turns: turn + 1,
                        experiment_count: experiments.len(),
                    });
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
                        self.emit_event(PlannerEvent::ToolCallStarted {
                            name: tool_call.name.clone(),
                            arguments: tool_call.arguments.clone(),
                        });
                        if self.verbose && self.event_tx.is_none() {
                            eprintln!("[tool] {}()", tool_call.name);
                        }

                        let mut result = self
                            .tool_registry
                            .execute(&tool_call.name, tool_call.arguments.clone())
                            .await;
                        result.tool_call_id = tool_call.id.clone();

                        self.emit_event(PlannerEvent::ToolCallCompleted {
                            name: tool_call.name.clone(),
                            result: result.content.clone(),
                            is_error: result.is_error,
                        });

                        // Capture target configs from discover_resources calls
                        if tool_call.name == "discover_resources" {
                            if let (Some(target), Some(config)) = (
                                tool_call.arguments["target"].as_str(),
                                tool_call.arguments.get("target_config"),
                            ) {
                                discovered_targets
                                    .insert(target.to_string(), config.clone());
                            }
                            // Emit discovery event with resource count
                            let resource_count = result
                                .content
                                .parse::<serde_json::Value>()
                                .ok()
                                .and_then(|v| v["total_resources"].as_u64())
                                .unwrap_or(0) as usize;
                            self.emit_event(PlannerEvent::DiscoveryResult {
                                target: tool_call.arguments["target"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                resource_count,
                            });
                        }

                        // Intercept run_experiment calls to capture experiment configs
                        if tool_call.name == "run_experiment" {
                            let mut exp_args = tool_call.arguments.clone();

                            // Auto-inject target_config if missing or null
                            let has_target_config = exp_args
                                .get("target_config")
                                .map_or(false, |v| !v.is_null() && v.is_object());
                            if !has_target_config {
                                let target_key = exp_args["target"]
                                    .as_str()
                                    .map(|s| s.to_string());

                                // Try exact target match first
                                let injected = target_key.as_ref().and_then(|t| {
                                    discovered_targets.get(t).cloned()
                                });

                                // Fallback: use the only discovered target if there's exactly one
                                let config_to_inject = injected.or_else(|| {
                                    if discovered_targets.len() == 1 {
                                        discovered_targets.values().next().cloned()
                                    } else {
                                        None
                                    }
                                });

                                if let Some(config) = config_to_inject {
                                    exp_args["target_config"] = config;
                                    if self.verbose && self.event_tx.is_none() {
                                        eprintln!("[planner] Auto-injected target_config from prior discovery");
                                    }
                                } else if self.verbose && self.event_tx.is_none() {
                                    eprintln!("[planner] Warning: no target_config available to inject");
                                }
                            }

                            let exp_name = tool_call.arguments["name"]
                                .as_str()
                                .unwrap_or("unnamed")
                                .to_string();
                            let exp_target = tool_call.arguments["target"]
                                .as_str()
                                .unwrap_or("unknown")
                                .to_string();
                            self.emit_event(PlannerEvent::ExperimentPlanned {
                                name: exp_name.clone(),
                                target: exp_target,
                            });
                            if self.verbose && self.event_tx.is_none() {
                                eprintln!("[experiment] Planned: {}", exp_name);
                            }
                            experiments.push(exp_args);
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
                    self.emit_event(PlannerEvent::PlanningComplete {
                        turns: turn + 1,
                        experiment_count: experiments.len(),
                    });
                    return Ok(PlanResult {
                        message: response.message.content,
                        experiments,
                        turns: turn + 1,
                    });
                }
                FinishReason::Other(reason) => {
                    tracing::warn!(reason = %reason, "Unexpected finish reason");
                    self.emit_event(PlannerEvent::PlanningComplete {
                        turns: turn + 1,
                        experiment_count: experiments.len(),
                    });
                    return Ok(PlanResult {
                        message: response.message.content,
                        experiments,
                        turns: turn + 1,
                    });
                }
            }
        }

        self.emit_event(PlannerEvent::PlanningComplete {
            turns: self.max_turns,
            experiment_count: experiments.len(),
        });
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
