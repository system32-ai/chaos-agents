use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sTargetConfig {
    /// Path to kubeconfig. If None, uses in-cluster config or default.
    pub kubeconfig: Option<String>,
    /// Target namespace. Defaults to "default".
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Label selector to filter target resources, e.g. "app=web"
    #[serde(default)]
    pub label_selector: Option<String>,
}

fn default_namespace() -> String {
    "default".to_string()
}
