use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoTargetConfig {
    pub connection_url: String,
    /// Optional: only target these databases. If empty, discover all.
    #[serde(default)]
    pub databases: Vec<String>,
}
