use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbTargetConfig {
    pub connection_url: String,
    pub db_type: DbType,
    /// Optional: only target these schemas. If empty, discover all.
    #[serde(default)]
    pub schemas: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbType {
    Postgres,
    Mysql,
}
