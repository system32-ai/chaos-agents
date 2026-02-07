use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum ExperimentEvent {
    Started {
        experiment_id: Uuid,
        at: DateTime<Utc>,
    },
    SkillExecuted {
        experiment_id: Uuid,
        skill_name: String,
        success: bool,
    },
    DurationWaitBegin {
        experiment_id: Uuid,
        duration: std::time::Duration,
    },
    RollbackStarted {
        experiment_id: Uuid,
    },
    RollbackStepCompleted {
        experiment_id: Uuid,
        skill_name: String,
        success: bool,
    },
    Completed {
        experiment_id: Uuid,
        at: DateTime<Utc>,
    },
    Failed {
        experiment_id: Uuid,
        error: String,
    },
}

/// Sink for experiment events.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: ExperimentEvent);
}

/// Channel-based event sink that forwards events to a receiver.
pub struct ChannelEventSink {
    tx: tokio::sync::mpsc::UnboundedSender<ExperimentEvent>,
}

impl ChannelEventSink {
    pub fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<ExperimentEvent>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

#[async_trait]
impl EventSink for ChannelEventSink {
    async fn emit(&self, event: ExperimentEvent) {
        let _ = self.tx.send(event);
    }
}

/// Simple tracing-based event sink.
pub struct TracingEventSink;

#[async_trait]
impl EventSink for TracingEventSink {
    async fn emit(&self, event: ExperimentEvent) {
        tracing::info!(?event, "experiment_event");
    }
}
