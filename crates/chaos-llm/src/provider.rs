use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::tool::ToolDefinition;

/// Configuration for selecting an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum LlmProviderConfig {
    Anthropic {
        api_key: String,
        #[serde(default = "default_anthropic_model")]
        model: String,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
    },
    Openai {
        api_key: String,
        #[serde(default = "default_openai_model")]
        model: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
    },
    Ollama {
        #[serde(default = "default_ollama_url")]
        base_url: String,
        model: String,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
    },
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-5-20250929".to_string()
}
fn default_openai_model() -> String {
    "gpt-4o".to_string()
}
fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}
fn default_max_tokens() -> u32 {
    4096
}

/// A message in a conversation with the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Tool calls requested by the assistant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Tool result (when role is Tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Response from an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub message: ChatMessage,
    pub finish_reason: FinishReason,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    ToolUse,
    MaxTokens,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// A unified interface for LLM providers.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request with optional tool definitions.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse>;

    /// Provider name for logging.
    fn name(&self) -> &str;
}

/// Anthropic Claude provider.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            max_tokens,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let system_msg = messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        let api_messages: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                if m.role == Role::Tool {
                    serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": m.tool_call_id,
                            "content": m.content,
                        }]
                    })
                } else if !m.tool_calls.is_empty() {
                    let content: Vec<serde_json::Value> = std::iter::once(
                        serde_json::json!({ "type": "text", "text": m.content })
                    )
                    .chain(m.tool_calls.iter().map(|tc| {
                        serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        })
                    }))
                    .collect();
                    serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    })
                } else {
                    serde_json::json!({
                        "role": match m.role {
                            Role::User => "user",
                            Role::Assistant => "assistant",
                            _ => "user",
                        },
                        "content": m.content,
                    })
                }
            })
            .collect();

        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
        });

        if let Some(sys) = system_msg {
            body["system"] = serde_json::json!(sys);
        }
        if !api_tools.is_empty() {
            body["tools"] = serde_json::json!(api_tools);
        }

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let response_body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!(
                "Anthropic API error ({}): {}",
                status,
                response_body
            );
        }

        parse_anthropic_response(&response_body)
    }
}

fn parse_anthropic_response(body: &serde_json::Value) -> anyhow::Result<LlmResponse> {
    let empty = vec![];
    let content = body["content"].as_array().unwrap_or(&empty);
    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in content {
        match block["type"].as_str() {
            Some("text") => {
                text.push_str(block["text"].as_str().unwrap_or(""));
            }
            Some("tool_use") => {
                tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    arguments: block["input"].clone(),
                });
            }
            _ => {}
        }
    }

    let stop_reason = body["stop_reason"].as_str().unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "end_turn" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolUse,
        "max_tokens" => FinishReason::MaxTokens,
        other => FinishReason::Other(other.to_string()),
    };

    let usage = body.get("usage").map(|u| TokenUsage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(LlmResponse {
        message: ChatMessage {
            role: Role::Assistant,
            content: text,
            tool_calls,
            tool_call_id: None,
        },
        finish_reason,
        usage,
    })
}

/// OpenAI-compatible provider (works with OpenAI, Azure OpenAI, and compatible APIs).
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String, base_url: Option<String>, max_tokens: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            max_tokens,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let api_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                if m.role == Role::Tool {
                    serde_json::json!({
                        "role": "tool",
                        "content": m.content,
                        "tool_call_id": m.tool_call_id,
                    })
                } else if !m.tool_calls.is_empty() {
                    serde_json::json!({
                        "role": "assistant",
                        "content": if m.content.is_empty() { serde_json::Value::Null } else { serde_json::json!(m.content) },
                        "tool_calls": m.tool_calls.iter().map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        }).collect::<Vec<_>>(),
                    })
                } else {
                    serde_json::json!({
                        "role": match m.role {
                            Role::System => "system",
                            Role::User => "user",
                            Role::Assistant => "assistant",
                            _ => "user",
                        },
                        "content": m.content,
                    })
                }
            })
            .collect();

        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
        });

        if !api_tools.is_empty() {
            body["tools"] = serde_json::json!(api_tools);
        }

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let response_body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!("OpenAI API error ({}): {}", status, response_body);
        }

        parse_openai_response(&response_body)
    }
}

fn parse_openai_response(body: &serde_json::Value) -> anyhow::Result<LlmResponse> {
    let choice = body["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or("").to_string();

    let tool_calls: Vec<ToolCall> = message["tool_calls"]
        .as_array()
        .map(|tcs| {
            tcs.iter()
                .filter_map(|tc| {
                    let id = tc["id"].as_str()?.to_string();
                    let name = tc["function"]["name"].as_str()?.to_string();
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments: serde_json::Value =
                        serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let finish_reason = match choice["finish_reason"].as_str() {
        Some("stop") => FinishReason::Stop,
        Some("tool_calls") => FinishReason::ToolUse,
        Some("length") => FinishReason::MaxTokens,
        Some(other) => FinishReason::Other(other.to_string()),
        None => FinishReason::Stop,
    };

    let usage = body.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(LlmResponse {
        message: ChatMessage {
            role: Role::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
        },
        finish_reason,
        usage,
    })
}

/// Ollama provider (local LLM inference).
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl OllamaProvider {
    pub fn new(base_url: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
            max_tokens,
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        // Ollama uses OpenAI-compatible API
        let api_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::Tool => "tool",
                    },
                    "content": m.content,
                })
            })
            .collect();

        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": api_messages,
            "stream": false,
            "options": {
                "num_predict": self.max_tokens,
            }
        });

        if !api_tools.is_empty() {
            body["tools"] = serde_json::json!(api_tools);
        }

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let response_body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!("Ollama API error ({}): {}", status, response_body);
        }

        let content = response_body["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let tool_calls: Vec<ToolCall> = response_body["message"]["tool_calls"]
            .as_array()
            .map(|tcs| {
                tcs.iter()
                    .enumerate()
                    .filter_map(|(i, tc)| {
                        let name = tc["function"]["name"].as_str()?.to_string();
                        let arguments = tc["function"]["arguments"].clone();
                        Some(ToolCall {
                            id: format!("call_{i}"),
                            name,
                            arguments,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            FinishReason::Stop
        };

        Ok(LlmResponse {
            message: ChatMessage {
                role: Role::Assistant,
                content,
                tool_calls,
                tool_call_id: None,
            },
            finish_reason,
            usage: None,
        })
    }
}

/// Create an LLM provider from config.
pub fn create_provider(config: &LlmProviderConfig) -> Box<dyn LlmProvider> {
    match config {
        LlmProviderConfig::Anthropic {
            api_key,
            model,
            max_tokens,
        } => Box::new(AnthropicProvider::new(
            api_key.clone(),
            model.clone(),
            *max_tokens,
        )),
        LlmProviderConfig::Openai {
            api_key,
            model,
            base_url,
            max_tokens,
        } => Box::new(OpenAiProvider::new(
            api_key.clone(),
            model.clone(),
            base_url.clone(),
            *max_tokens,
        )),
        LlmProviderConfig::Ollama {
            base_url,
            model,
            max_tokens,
        } => Box::new(OllamaProvider::new(
            base_url.clone(),
            model.clone(),
            *max_tokens,
        )),
    }
}
