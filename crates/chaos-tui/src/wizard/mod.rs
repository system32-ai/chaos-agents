pub mod welcome;
pub mod provider;
pub mod provider_config;
pub mod target;
pub mod target_config;
pub mod prompt;
pub mod review;

use chaos_llm::provider::LlmProviderConfig;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;

use crate::widgets::input::TextInput;
use crate::widgets::selector::{Selector, SelectorItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardScreen {
    Welcome,
    SelectProvider,
    ConfigureProvider,
    SelectTarget,
    ConfigureTarget,
    EnterPrompt,
    Review,
}

pub enum WizardTransition {
    Stay,
    Next(WizardScreen),
    Back(WizardScreen),
    Quit,
    StartExecution,
}

#[derive(Clone)]
pub struct WizardOutput {
    pub provider_config: LlmProviderConfig,
    pub prompt: String,
    pub max_turns: u32,
    pub duration: String,
}

pub struct WizardState {
    pub screen: WizardScreen,
    // Provider selection
    pub provider_selector: Selector,
    pub selected_provider: Option<String>,
    // Provider config fields
    pub api_key_input: TextInput,
    pub model_input: TextInput,
    pub base_url_input: TextInput,
    pub max_turns_input: TextInput,
    pub provider_field_index: usize,
    // Target selection
    pub target_selector: Selector,
    pub selected_target: Option<String>,
    // Database fields
    pub db_url_input: TextInput,
    pub db_type_selector: Selector,
    pub db_schemas_input: TextInput,
    // Kubernetes fields
    pub k8s_namespace_input: TextInput,
    pub k8s_label_input: TextInput,
    pub k8s_kubeconfig_input: TextInput,
    // Server fields
    pub server_host_input: TextInput,
    pub server_port_input: TextInput,
    pub server_username_input: TextInput,
    pub server_auth_selector: Selector,
    pub server_auth_value_input: TextInput,
    // Target config field index
    pub target_field_index: usize,
    // Prompt
    pub prompt_input: TextInput,
    pub duration_input: TextInput,
    // Error
    pub error_message: Option<String>,
}

impl WizardState {
    pub fn new() -> Self {
        let anthropic_detected = std::env::var("ANTHROPIC_API_KEY").is_ok();
        let openai_detected = std::env::var("OPENAI_API_KEY").is_ok();

        let provider_selector = Selector::new(
            " Select Provider ",
            vec![
                SelectorItem {
                    label: "Anthropic".into(),
                    description: "Claude models (claude-sonnet-4-5, etc.)".into(),
                    hint: if anthropic_detected {
                        Some("API key detected".into())
                    } else {
                        None
                    },
                },
                SelectorItem {
                    label: "OpenAI".into(),
                    description: "GPT models (gpt-4o, etc.)".into(),
                    hint: if openai_detected {
                        Some("API key detected".into())
                    } else {
                        None
                    },
                },
                SelectorItem {
                    label: "Ollama".into(),
                    description: "Local models (llama3.1, etc.)".into(),
                    hint: Some("No API key needed".into()),
                },
            ],
        );

        let target_selector = Selector::new(
            " Select Target ",
            vec![
                SelectorItem {
                    label: "Database".into(),
                    description: "PostgreSQL, MySQL, MongoDB".into(),
                    hint: None,
                },
                SelectorItem {
                    label: "Kubernetes".into(),
                    description: "Pods, services, deployments".into(),
                    hint: None,
                },
                SelectorItem {
                    label: "Server".into(),
                    description: "SSH-accessible Linux servers".into(),
                    hint: None,
                },
            ],
        );

        let db_type_selector = Selector::new(
            " Database Type ",
            vec![
                SelectorItem {
                    label: "postgres".into(),
                    description: "PostgreSQL".into(),
                    hint: None,
                },
                SelectorItem {
                    label: "mysql".into(),
                    description: "MySQL".into(),
                    hint: None,
                },
                SelectorItem {
                    label: "mongodb".into(),
                    description: "MongoDB".into(),
                    hint: None,
                },
            ],
        );

        let server_auth_selector = Selector::new(
            " Auth Type ",
            vec![
                SelectorItem {
                    label: "key".into(),
                    description: "SSH private key".into(),
                    hint: None,
                },
                SelectorItem {
                    label: "password".into(),
                    description: "Password authentication".into(),
                    hint: None,
                },
            ],
        );

        // Pre-fill from env vars
        let api_key_prefill = std::env::var("ANTHROPIC_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_default();
        let kubeconfig_prefill = std::env::var("KUBECONFIG").unwrap_or_default();
        let default_key_path = dirs_home().map(|h| format!("{h}/.ssh/id_ed25519")).unwrap_or_default();

        Self {
            screen: WizardScreen::Welcome,
            provider_selector,
            selected_provider: None,
            api_key_input: TextInput::new(" API Key ").with_masked().with_content(&api_key_prefill),
            model_input: TextInput::new(" Model "),
            base_url_input: TextInput::new(" Base URL "),
            max_turns_input: TextInput::new(" Max Turns ").with_content("10"),
            provider_field_index: 0,
            target_selector,
            selected_target: None,
            db_url_input: TextInput::new(" Connection URL "),
            db_type_selector,
            db_schemas_input: TextInput::new(" Schemas (comma-separated) "),
            k8s_namespace_input: TextInput::new(" Namespace ").with_content("default"),
            k8s_label_input: TextInput::new(" Label Selector "),
            k8s_kubeconfig_input: TextInput::new(" Kubeconfig Path ").with_content(&kubeconfig_prefill),
            server_host_input: TextInput::new(" Host "),
            server_port_input: TextInput::new(" Port ").with_content("22"),
            server_username_input: TextInput::new(" Username "),
            server_auth_selector,
            server_auth_value_input: TextInput::new(" Key Path ").with_content(&default_key_path),
            target_field_index: 0,
            prompt_input: TextInput::new(" Chaos Prompt ").with_multiline(),
            duration_input: TextInput::new(" Duration ").with_content("5m"),
            error_message: None,
        }
    }

    pub fn into_output(&self) -> anyhow::Result<WizardOutput> {
        let provider = self
            .selected_provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No provider selected"))?;

        let provider_config = match provider.as_str() {
            "anthropic" => LlmProviderConfig::Anthropic {
                api_key: self.api_key_input.content.clone(),
                model: if self.model_input.content.is_empty() {
                    "claude-sonnet-4-5-20250929".to_string()
                } else {
                    self.model_input.content.clone()
                },
                max_tokens: 4096,
            },
            "openai" => LlmProviderConfig::Openai {
                api_key: self.api_key_input.content.clone(),
                model: if self.model_input.content.is_empty() {
                    "gpt-4o".to_string()
                } else {
                    self.model_input.content.clone()
                },
                base_url: if self.base_url_input.content.is_empty() {
                    None
                } else {
                    Some(self.base_url_input.content.clone())
                },
                max_tokens: 4096,
            },
            "ollama" => LlmProviderConfig::Ollama {
                base_url: if self.base_url_input.content.is_empty() {
                    "http://localhost:11434".to_string()
                } else {
                    self.base_url_input.content.clone()
                },
                model: if self.model_input.content.is_empty() {
                    "llama3.1".to_string()
                } else {
                    self.model_input.content.clone()
                },
                max_tokens: 4096,
            },
            _ => anyhow::bail!("Unknown provider: {provider}"),
        };

        let max_turns = self
            .max_turns_input
            .content
            .trim()
            .parse::<u32>()
            .unwrap_or(10);

        let duration = if self.duration_input.content.trim().is_empty() {
            "5m".to_string()
        } else {
            self.duration_input.content.trim().to_string()
        };

        Ok(WizardOutput {
            provider_config,
            prompt: self.prompt_input.content.clone(),
            max_turns,
            duration,
        })
    }
}

pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    match state.screen {
        WizardScreen::Welcome => welcome::render(state, frame, area),
        WizardScreen::SelectProvider => provider::render(state, frame, area),
        WizardScreen::ConfigureProvider => provider_config::render(state, frame, area),
        WizardScreen::SelectTarget => target::render(state, frame, area),
        WizardScreen::ConfigureTarget => target_config::render(state, frame, area),
        WizardScreen::EnterPrompt => prompt::render(state, frame, area),
        WizardScreen::Review => review::render(state, frame, area),
    }
}

pub fn handle_key(state: &mut WizardState, key: KeyEvent) -> WizardTransition {
    // Global: Ctrl+C or Ctrl+X quits from wizard
    if (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('x'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return WizardTransition::Quit;
    }

    // Global: Esc goes back
    if key.code == KeyCode::Esc {
        return match state.screen {
            WizardScreen::Welcome => WizardTransition::Quit,
            WizardScreen::SelectProvider => {
                state.screen = WizardScreen::Welcome;
                WizardTransition::Back(WizardScreen::Welcome)
            }
            WizardScreen::ConfigureProvider => {
                state.screen = WizardScreen::SelectProvider;
                WizardTransition::Back(WizardScreen::SelectProvider)
            }
            WizardScreen::SelectTarget | WizardScreen::ConfigureTarget => {
                state.screen = WizardScreen::ConfigureProvider;
                WizardTransition::Back(WizardScreen::ConfigureProvider)
            }
            WizardScreen::EnterPrompt => {
                state.screen = WizardScreen::ConfigureProvider;
                WizardTransition::Back(WizardScreen::ConfigureProvider)
            }
            WizardScreen::Review => {
                state.screen = WizardScreen::EnterPrompt;
                WizardTransition::Back(WizardScreen::EnterPrompt)
            }
        };
    }

    match state.screen {
        WizardScreen::Welcome => welcome::handle_key(state, key),
        WizardScreen::SelectProvider => provider::handle_key(state, key),
        WizardScreen::ConfigureProvider => provider_config::handle_key(state, key),
        WizardScreen::SelectTarget => target::handle_key(state, key),
        WizardScreen::ConfigureTarget => target_config::handle_key(state, key),
        WizardScreen::EnterPrompt => prompt::handle_key(state, key),
        WizardScreen::Review => review::handle_key(state, key),
    }
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok()
}
