use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Definition of a tool that the LLM can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the parameters.
    pub parameters: serde_json::Value,
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

/// A callable tool that can be executed by the agent.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool definition for the LLM.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with given arguments.
    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String>;
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.definition().name.clone();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, name: &str, arguments: serde_json::Value) -> ToolResult {
        match self.tools.get(name) {
            Some(tool) => match tool.execute(arguments).await {
                Ok(content) => ToolResult {
                    tool_call_id: String::new(),
                    content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_call_id: String::new(),
                    content: format!("Error: {e}"),
                    is_error: true,
                },
            },
            None => ToolResult {
                tool_call_id: String::new(),
                content: format!("Unknown tool: {name}"),
                is_error: true,
            },
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Built-in tools that expose chaos skills to the LLM ──

/// Tool that lists available chaos skills.
pub struct ListSkillsTool {
    pub skills: Vec<ToolDefinition>,
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_skills".into(),
            description: "List all available chaos engineering skills".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["database", "kubernetes", "server"],
                        "description": "Filter by target domain"
                    }
                }
            }),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        let filter = arguments["target"].as_str();
        let filtered: Vec<_> = self
            .skills
            .iter()
            .filter(|s| {
                filter.map_or(true, |f| s.name.starts_with(f) || s.description.to_lowercase().contains(f))
            })
            .collect();
        Ok(serde_json::to_string_pretty(&filtered)?)
    }
}

/// Tool that runs a chaos experiment.
pub struct RunExperimentTool;

#[async_trait]
impl Tool for RunExperimentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_experiment".into(),
            description: "Run a chaos experiment with specified skills and target configuration".into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["name", "target", "target_config", "skills", "duration"],
                "properties": {
                    "name": { "type": "string", "description": "Experiment name" },
                    "target": { "type": "string", "enum": ["database", "kubernetes", "server"] },
                    "target_config": {
                        "type": "object",
                        "description": "Target connection config. MUST reuse the exact same config you passed to discover_resources. For database: {\"connection_url\": \"postgres://user:pass@host:5432/db\", \"db_type\": \"postgres\"} (db_type values: postgres, mysql, cockroach_db, yugabyte_db, mongo_d_b). For kubernetes: {\"namespace\": \"default\", \"label_selector\": \"app=web\"}. For server: {\"hosts\": [{\"host\": \"1.2.3.4\", \"port\": 22, \"username\": \"user\", \"auth\": {\"type\": \"key\", \"private_key_path\": \"~/.ssh/id_ed25519\"}}]}"
                    },
                    "skills": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["skill_name"],
                            "properties": {
                                "skill_name": { "type": "string" },
                                "params": { "type": "object" },
                                "count": { "type": "integer", "default": 1 }
                            }
                        }
                    },
                    "duration": { "type": "string", "description": "Chaos duration, e.g. '5m', '1h'" }
                }
            }),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        // This is a placeholder. The actual execution is handled by the planner
        // which intercepts this tool call and dispatches it to the orchestrator.
        Ok(format!(
            "Experiment '{}' submitted for execution",
            arguments["name"].as_str().unwrap_or("unnamed")
        ))
    }
}

/// Tool that discovers resources on a target.
pub struct DiscoverResourcesTool;

#[async_trait]
impl Tool for DiscoverResourcesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "discover_resources".into(),
            description: "Discover resources (tables, pods, services) on a chaos target".into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["target", "target_config"],
                "properties": {
                    "target": { "type": "string", "enum": ["database", "kubernetes", "server"] },
                    "target_config": { "type": "object", "description": "Target-specific configuration" }
                }
            }),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        Ok(format!(
            "Discovery submitted for target: {}",
            arguments["target"].as_str().unwrap_or("unknown")
        ))
    }
}
