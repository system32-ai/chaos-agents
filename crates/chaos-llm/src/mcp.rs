use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::tool::{Tool, ToolDefinition, ToolRegistry};

/// MCP (Model Context Protocol) server configuration.
/// Allows connecting to external MCP servers that provide additional tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name of the MCP server.
    pub name: String,
    /// Transport type.
    pub transport: McpTransport,
    /// Environment variables to pass to the MCP server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// stdio-based MCP server (command + args).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// SSE-based MCP server.
    Sse { url: String },
}

/// An MCP client that connects to an MCP server and exposes its tools.
pub struct McpClient {
    config: McpServerConfig,
    tools: Vec<McpToolProxy>,
}

impl McpClient {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            tools: Vec::new(),
        }
    }

    /// Initialize the MCP connection and discover available tools.
    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        match &self.config.transport {
            McpTransport::Stdio { command, args } => {
                tracing::info!(
                    name = %self.config.name,
                    command = %command,
                    "Initializing stdio MCP server"
                );
                // In a full implementation, this would:
                // 1. Spawn the child process
                // 2. Send initialize request via JSON-RPC over stdin/stdout
                // 3. Call tools/list to discover available tools
                // 4. Create McpToolProxy for each discovered tool

                // For now, log the intent - the actual MCP protocol implementation
                // would use the JSON-RPC protocol over stdio.
                tracing::info!(
                    name = %self.config.name,
                    command = %command,
                    args = ?args,
                    "MCP stdio server configured (connect on first tool call)"
                );
            }
            McpTransport::Sse { url } => {
                tracing::info!(
                    name = %self.config.name,
                    url = %url,
                    "Initializing SSE MCP server"
                );
                // In a full implementation, this would:
                // 1. Connect to the SSE endpoint
                // 2. Send initialize request
                // 3. Discover tools
            }
        }
        Ok(())
    }

    /// Register all discovered MCP tools into a ToolRegistry.
    pub fn register_tools(&self, registry: &mut ToolRegistry) {
        for tool in &self.tools {
            registry.register(Box::new(tool.clone()));
        }
    }

    /// Get tool definitions from this MCP server.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.definition.clone()).collect()
    }
}

/// A proxy tool that forwards calls to an MCP server.
#[derive(Clone)]
pub struct McpToolProxy {
    pub server_name: String,
    pub definition: ToolDefinition,
    transport: McpTransportHandle,
}

#[derive(Clone)]
enum McpTransportHandle {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
    },
}

impl McpToolProxy {
    pub fn new_stdio(
        server_name: String,
        definition: ToolDefinition,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            server_name,
            definition,
            transport: McpTransportHandle::Stdio { command, args, env },
        }
    }

    pub fn new_sse(server_name: String, definition: ToolDefinition, url: String) -> Self {
        Self {
            server_name,
            definition,
            transport: McpTransportHandle::Sse { url },
        }
    }

    async fn call_stdio(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> anyhow::Result<String> {
        use tokio::process::Command;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
            }
        });

        let request_str = serde_json::to_string(&request)?;

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // Write request to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(request_str.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            drop(stdin);
        }

        let output = child.wait_with_output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON-RPC response
        if let Ok(response) = serde_json::from_str::<serde_json::Value>(&stdout) {
            if let Some(result) = response.get("result") {
                if let Some(content) = result["content"].as_array() {
                    let text: String = content
                        .iter()
                        .filter_map(|c| c["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Ok(text);
                }
                return Ok(result.to_string());
            }
            if let Some(error) = response.get("error") {
                anyhow::bail!("MCP error: {}", error);
            }
        }

        Ok(stdout.to_string())
    }

    async fn call_sse(
        &self,
        url: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> anyhow::Result<String> {
        let client = reqwest::Client::new();
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
            }
        });

        let resp = client
            .post(url)
            .json(&request)
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        if let Some(result) = body.get("result") {
            Ok(result.to_string())
        } else if let Some(error) = body.get("error") {
            anyhow::bail!("MCP error: {}", error)
        } else {
            Ok(body.to_string())
        }
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<String> {
        tracing::info!(
            server = %self.server_name,
            tool = %self.definition.name,
            "Calling MCP tool"
        );

        match &self.transport {
            McpTransportHandle::Stdio { command, args, env } => {
                self.call_stdio(command, args, env, &self.definition.name, &arguments)
                    .await
            }
            McpTransportHandle::Sse { url } => {
                self.call_sse(url, &self.definition.name, &arguments).await
            }
        }
    }
}
