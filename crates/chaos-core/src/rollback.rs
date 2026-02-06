use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque blob capturing what a skill needs to undo its action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackHandle {
    pub id: Uuid,
    pub skill_name: String,
    pub created_at: DateTime<Utc>,
    /// Skill-specific serialized undo state.
    pub undo_state: serde_yaml::Value,
}

impl RollbackHandle {
    pub fn new(skill_name: impl Into<String>, undo_state: serde_yaml::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            skill_name: skill_name.into(),
            created_at: Utc::now(),
            undo_state,
        }
    }
}

/// Ordered log of rollback handles for an experiment.
/// Rollback pops in LIFO (reverse) order.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RollbackLog {
    entries: Vec<RollbackHandle>,
}

impl RollbackLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, handle: RollbackHandle) {
        self.entries.push(handle);
    }

    /// Returns an iterator from most-recent to oldest (LIFO).
    pub fn iter_reverse(&self) -> impl Iterator<Item = &RollbackHandle> {
        self.entries.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
