use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChaosError {
    #[error("Skill execution failed: {skill_name} -- {source}")]
    SkillExecution {
        skill_name: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Rollback failed: {skill_name} -- {source}")]
    RollbackFailed {
        skill_name: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(#[source] anyhow::Error),

    #[error("Discovery failed: {0}")]
    Discovery(String),

    #[error("Experiment timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type ChaosResult<T> = Result<T, ChaosError>;
