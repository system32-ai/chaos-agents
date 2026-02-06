use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::Agent;
use crate::error::{ChaosError, ChaosResult};
use crate::event::{EventSink, ExperimentEvent};
use crate::experiment::{Experiment, ExperimentConfig, ExperimentStatus};
use crate::skill::TargetDomain;

pub struct Orchestrator {
    agents: HashMap<TargetDomain, Arc<RwLock<Box<dyn Agent>>>>,
    experiments: Arc<RwLock<HashMap<Uuid, Experiment>>>,
    event_sinks: Vec<Arc<dyn EventSink>>,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            experiments: Arc::new(RwLock::new(HashMap::new())),
            event_sinks: Vec::new(),
        }
    }

    pub fn register_agent(&mut self, agent: Box<dyn Agent>) {
        let domain = agent.domain();
        self.agents.insert(domain, Arc::new(RwLock::new(agent)));
    }

    pub fn add_event_sink(&mut self, sink: Arc<dyn EventSink>) {
        self.event_sinks.push(sink);
    }

    async fn emit(&self, event: ExperimentEvent) {
        for sink in &self.event_sinks {
            sink.emit(event.clone()).await;
        }
    }

    /// Run a single experiment to completion (execute -> wait duration -> rollback).
    pub async fn run_experiment(&self, config: ExperimentConfig) -> ChaosResult<Uuid> {
        let agent_lock = self
            .agents
            .get(&config.target)
            .ok_or_else(|| {
                ChaosError::Config(format!("No agent registered for target: {}", config.target))
            })?
            .clone();

        let mut experiment = Experiment::new(config.clone());
        let experiment_id = experiment.id;

        self.emit(ExperimentEvent::Started {
            experiment_id,
            at: chrono::Utc::now(),
        })
        .await;

        // Initialize agent
        {
            let mut agent = agent_lock.write().await;
            agent.initialize().await?;
        }

        // Discovery phase
        experiment.status = ExperimentStatus::Discovering;
        {
            let mut agent = agent_lock.write().await;
            let resources = agent.discover().await?;
            tracing::info!(
                count = resources.len(),
                "Discovered resources on target"
            );
        }

        // Execution phase
        experiment.status = ExperimentStatus::Executing;
        experiment.started_at = Some(chrono::Utc::now());

        let execution_result = self
            .execute_skills(&agent_lock, &mut experiment)
            .await;

        if let Err(ref e) = execution_result {
            tracing::error!(error = %e, "Skill execution failed, initiating rollback");
            self.emit(ExperimentEvent::Failed {
                experiment_id,
                error: e.to_string(),
            })
            .await;
        }

        // Wait for configured duration (soak period)
        if execution_result.is_ok() {
            experiment.status = ExperimentStatus::WaitingDuration;
            self.emit(ExperimentEvent::DurationWaitBegin {
                experiment_id,
                duration: config.duration,
            })
            .await;
            tracing::info!(duration = ?config.duration, "Waiting for chaos duration");
            tokio::time::sleep(config.duration).await;
        }

        // Rollback phase (always runs)
        experiment.status = ExperimentStatus::RollingBack;
        self.emit(ExperimentEvent::RollbackStarted { experiment_id })
            .await;

        self.rollback_experiment(&agent_lock, &mut experiment).await;

        // Complete
        experiment.status = ExperimentStatus::Completed;
        experiment.completed_at = Some(chrono::Utc::now());

        self.emit(ExperimentEvent::Completed {
            experiment_id,
            at: chrono::Utc::now(),
        })
        .await;

        // Store experiment
        self.experiments
            .write()
            .await
            .insert(experiment_id, experiment);

        Ok(experiment_id)
    }

    async fn execute_skills(
        &self,
        agent_lock: &Arc<RwLock<Box<dyn Agent>>>,
        experiment: &mut Experiment,
    ) -> ChaosResult<()> {
        let agent = agent_lock.read().await;

        for invocation in &experiment.config.skills {
            let skill = agent.skill_by_name(&invocation.skill_name).ok_or_else(|| {
                ChaosError::Config(format!("Unknown skill: {}", invocation.skill_name))
            })?;

            skill.validate_params(&invocation.params)?;

            for _ in 0..invocation.count {
                let ctx = agent.build_context().await?;
                match skill.execute(&ctx).await {
                    Ok(handle) => {
                        tracing::info!(skill = %invocation.skill_name, "Skill executed successfully");
                        self.emit(ExperimentEvent::SkillExecuted {
                            experiment_id: experiment.id,
                            skill_name: invocation.skill_name.clone(),
                            success: true,
                        })
                        .await;
                        experiment.rollback_log.push(handle);
                    }
                    Err(e) => {
                        self.emit(ExperimentEvent::SkillExecuted {
                            experiment_id: experiment.id,
                            skill_name: invocation.skill_name.clone(),
                            success: false,
                        })
                        .await;
                        return Err(ChaosError::SkillExecution {
                            skill_name: invocation.skill_name.clone(),
                            source: e.into(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Rollback in LIFO order. Best-effort: continues even if individual rollbacks fail.
    async fn rollback_experiment(
        &self,
        agent_lock: &Arc<RwLock<Box<dyn Agent>>>,
        experiment: &mut Experiment,
    ) {
        let agent = agent_lock.read().await;

        let handles: Vec<_> = experiment.rollback_log.iter_reverse().cloned().collect();
        for handle in &handles {
            let skill = match agent.skill_by_name(&handle.skill_name) {
                Some(s) => s,
                None => {
                    tracing::error!(skill = %handle.skill_name, "Skill not found for rollback");
                    continue;
                }
            };

            let ctx = match agent.build_context().await {
                Ok(ctx) => ctx,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to build context for rollback");
                    continue;
                }
            };

            let success = match skill.rollback(&ctx, handle).await {
                Ok(()) => {
                    tracing::info!(skill = %handle.skill_name, "Rollback succeeded");
                    true
                }
                Err(e) => {
                    tracing::error!(skill = %handle.skill_name, error = %e, "Rollback failed");
                    false
                }
            };

            self.emit(ExperimentEvent::RollbackStepCompleted {
                experiment_id: experiment.id,
                skill_name: handle.skill_name.clone(),
                success,
            })
            .await;
        }
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
