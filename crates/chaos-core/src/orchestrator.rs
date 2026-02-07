use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::Agent;
use crate::error::{ChaosError, ChaosResult};
use crate::event::{EventSink, ExperimentEvent};
use crate::experiment::{Experiment, ExperimentConfig, ExperimentStatus};
use crate::report::{
    DiscoveredResourceSummary, ExperimentReport, RollbackStepRecord, SkillExecutionRecord,
};
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
    pub async fn run_experiment(
        &self,
        config: ExperimentConfig,
    ) -> ChaosResult<ExperimentReport> {
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
        let discovered_summaries: Vec<DiscoveredResourceSummary>;
        {
            let mut agent = agent_lock.write().await;
            let resources = agent.discover().await?;
            tracing::info!(
                count = resources.len(),
                "Discovered resources on target"
            );
            discovered_summaries = resources
                .iter()
                .map(|r| DiscoveredResourceSummary {
                    resource_type: r.resource_type().to_string(),
                    name: r.name().to_string(),
                })
                .collect();
        }

        // Execution phase
        experiment.status = ExperimentStatus::Executing;
        experiment.started_at = Some(chrono::Utc::now());

        let mut skill_records = Vec::new();
        let execution_result = self
            .execute_skills(&agent_lock, &mut experiment, &mut skill_records)
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

        let mut rollback_records = Vec::new();
        self.rollback_experiment(&agent_lock, &mut experiment, &mut rollback_records)
            .await;

        // Complete
        let failure_error = execution_result.err().map(|e| e.to_string());
        if let Some(ref err) = failure_error {
            experiment.status = ExperimentStatus::Failed(err.clone());
        } else {
            experiment.status = ExperimentStatus::Completed;
        }
        experiment.completed_at = Some(chrono::Utc::now());

        self.emit(ExperimentEvent::Completed {
            experiment_id,
            at: chrono::Utc::now(),
        })
        .await;

        // Build report
        let started_at = experiment.started_at.unwrap_or_else(chrono::Utc::now);
        let completed_at = experiment.completed_at.unwrap_or_else(chrono::Utc::now);
        let total_duration = (completed_at - started_at)
            .to_std()
            .unwrap_or_default();

        let report = ExperimentReport {
            experiment_id,
            experiment_name: config.name.clone(),
            target_domain: config.target,
            status: match &experiment.status {
                ExperimentStatus::Completed => "completed".to_string(),
                ExperimentStatus::Failed(e) => format!("failed: {e}"),
                other => format!("{other:?}"),
            },
            started_at,
            completed_at,
            total_duration,
            soak_duration: config.duration,
            discovered_resources: discovered_summaries,
            skill_executions: skill_records,
            rollback_steps: rollback_records,
        };

        // Store experiment
        self.experiments
            .write()
            .await
            .insert(experiment_id, experiment);

        Ok(report)
    }

    async fn execute_skills(
        &self,
        agent_lock: &Arc<RwLock<Box<dyn Agent>>>,
        experiment: &mut Experiment,
        records: &mut Vec<SkillExecutionRecord>,
    ) -> ChaosResult<()> {
        let agent = agent_lock.read().await;

        for invocation in &experiment.config.skills {
            let skill = agent.skill_by_name(&invocation.skill_name).ok_or_else(|| {
                ChaosError::Config(format!("Unknown skill: {}", invocation.skill_name))
            })?;

            skill.validate_params(&invocation.params)?;

            for _ in 0..invocation.count {
                let ctx = agent.build_context().await?;
                let start = Instant::now();
                match skill.execute(&ctx).await {
                    Ok(handle) => {
                        let elapsed = start.elapsed();
                        tracing::info!(skill = %invocation.skill_name, "Skill executed successfully");
                        self.emit(ExperimentEvent::SkillExecuted {
                            experiment_id: experiment.id,
                            skill_name: invocation.skill_name.clone(),
                            success: true,
                        })
                        .await;
                        experiment.rollback_log.push(handle);
                        records.push(SkillExecutionRecord {
                            skill_name: invocation.skill_name.clone(),
                            success: true,
                            duration: elapsed,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let elapsed = start.elapsed();
                        self.emit(ExperimentEvent::SkillExecuted {
                            experiment_id: experiment.id,
                            skill_name: invocation.skill_name.clone(),
                            success: false,
                        })
                        .await;
                        records.push(SkillExecutionRecord {
                            skill_name: invocation.skill_name.clone(),
                            success: false,
                            duration: elapsed,
                            error: Some(e.to_string()),
                        });
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
        rollback_records: &mut Vec<RollbackStepRecord>,
    ) {
        let agent = agent_lock.read().await;

        let handles: Vec<_> = experiment.rollback_log.iter_reverse().cloned().collect();
        for handle in &handles {
            let skill = match agent.skill_by_name(&handle.skill_name) {
                Some(s) => s,
                None => {
                    tracing::error!(skill = %handle.skill_name, "Skill not found for rollback");
                    rollback_records.push(RollbackStepRecord {
                        skill_name: handle.skill_name.clone(),
                        success: false,
                        duration: std::time::Duration::ZERO,
                        error: Some("skill not found".to_string()),
                    });
                    continue;
                }
            };

            let ctx = match agent.build_context().await {
                Ok(ctx) => ctx,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to build context for rollback");
                    rollback_records.push(RollbackStepRecord {
                        skill_name: handle.skill_name.clone(),
                        success: false,
                        duration: std::time::Duration::ZERO,
                        error: Some(format!("context build failed: {e}")),
                    });
                    continue;
                }
            };

            let start = Instant::now();
            let (success, error) = match skill.rollback(&ctx, handle).await {
                Ok(()) => {
                    tracing::info!(skill = %handle.skill_name, "Rollback succeeded");
                    (true, None)
                }
                Err(e) => {
                    tracing::error!(skill = %handle.skill_name, error = %e, "Rollback failed");
                    (false, Some(e.to_string()))
                }
            };
            let elapsed = start.elapsed();

            rollback_records.push(RollbackStepRecord {
                skill_name: handle.skill_name.clone(),
                success,
                duration: elapsed,
                error,
            });

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
