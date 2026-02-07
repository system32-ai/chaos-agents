use serde::{Deserialize, Serialize};
use std::fmt;

use crate::skill::TargetDomain;

/// A resource discovered on a target.
pub trait DiscoveredResource: Send + Sync + fmt::Debug {
    fn domain(&self) -> TargetDomain;
    fn resource_type(&self) -> &str;
    fn name(&self) -> &str;
    fn metadata(&self) -> serde_yaml::Value;
}

/// Concrete resource for database targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbResource {
    pub table_name: String,
    pub schema: String,
    pub columns: Vec<ColumnInfo>,
    pub row_count_estimate: u64,
}

impl DiscoveredResource for DbResource {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Database
    }
    fn resource_type(&self) -> &str {
        "table"
    }
    fn name(&self) -> &str {
        &self.table_name
    }
    fn metadata(&self) -> serde_yaml::Value {
        serde_yaml::to_value(self).unwrap_or(serde_yaml::Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
}

/// Concrete resource for MongoDB targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoResource {
    pub database: String,
    pub collection: String,
    pub document_count: u64,
}

impl DiscoveredResource for MongoResource {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Database
    }
    fn resource_type(&self) -> &str {
        "collection"
    }
    fn name(&self) -> &str {
        &self.collection
    }
    fn metadata(&self) -> serde_yaml::Value {
        serde_yaml::to_value(self).unwrap_or(serde_yaml::Value::Null)
    }
}

/// Concrete resource for Kubernetes targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sResource {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub labels: std::collections::HashMap<String, String>,
}

impl DiscoveredResource for K8sResource {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Kubernetes
    }
    fn resource_type(&self) -> &str {
        &self.kind
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn metadata(&self) -> serde_yaml::Value {
        serde_yaml::to_value(self).unwrap_or(serde_yaml::Value::Null)
    }
}

/// Concrete resource for server targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResource {
    pub host: String,
    pub resource_type: ServerResourceType,
    pub name: String,
    pub details: serde_yaml::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerResourceType {
    RunningService,
    ListeningPort,
    MountedFilesystem,
    Process,
}

impl DiscoveredResource for ServerResource {
    fn domain(&self) -> TargetDomain {
        TargetDomain::Server
    }
    fn resource_type(&self) -> &str {
        match &self.resource_type {
            ServerResourceType::RunningService => "service",
            ServerResourceType::ListeningPort => "port",
            ServerResourceType::MountedFilesystem => "filesystem",
            ServerResourceType::Process => "process",
        }
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn metadata(&self) -> serde_yaml::Value {
        serde_yaml::to_value(self).unwrap_or(serde_yaml::Value::Null)
    }
}
