use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use clap::Args;
use cron::Schedule;
use std::str::FromStr;
use tokio::sync::Semaphore;

use chaos_core::config::DaemonConfig;
use chaos_core::event::TracingEventSink;
use chaos_core::orchestrator::Orchestrator;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_k8s::agent::K8sAgent;
use chaos_server::agent::ServerAgent;

#[derive(Args)]
pub struct DaemonArgs {
    /// Path to the daemon schedule YAML config
    pub config: PathBuf,
    /// PID file for daemon management
    #[arg(long)]
    pub pid_file: Option<PathBuf>,
}

pub async fn execute(args: DaemonArgs) -> anyhow::Result<()> {
    let config = DaemonConfig::from_file(&args.config)?;

    tracing::info!(
        experiments = config.experiments.len(),
        max_concurrent = config.settings.max_concurrent,
        "Daemon starting"
    );

    // Validate all cron expressions upfront
    for scheduled in &config.experiments {
        Schedule::from_str(&scheduled.schedule)
            .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {e}", scheduled.schedule))?;
    }

    // Write PID file if requested
    if let Some(ref pid_path) = args.pid_file {
        std::fs::write(pid_path, std::process::id().to_string())?;
    }

    // Set up shutdown signal
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received shutdown signal");
        let _ = shutdown_tx.send(true);
    });

    let semaphore = Arc::new(Semaphore::new(config.settings.max_concurrent));
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut last_check = Utc::now();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Utc::now();

                for scheduled in &config.experiments {
                    if !scheduled.enabled {
                        continue;
                    }

                    let sched = Schedule::from_str(&scheduled.schedule).unwrap();
                    let has_trigger = sched
                        .after(&last_check)
                        .take_while(|t| t <= &now)
                        .next()
                        .is_some();

                    if has_trigger {
                        let permit = match semaphore.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(
                                    experiment = %scheduled.experiment.name,
                                    "Skipping: max concurrent experiments reached"
                                );
                                continue;
                            }
                        };

                        let exp_config = scheduled.experiment.clone();
                        let exp_name = exp_config.name.clone();

                        tokio::spawn(async move {
                            let _permit = permit;

                            // Create a fresh orchestrator for this experiment run
                            let mut orchestrator = Orchestrator::new();
                            orchestrator.add_event_sink(Arc::new(TracingEventSink));

                            match exp_config.target {
                                TargetDomain::Database => {
                                    if let Ok(agent) = DbAgent::from_yaml(&exp_config.target_config) {
                                        orchestrator.register_agent(Box::new(agent));
                                    }
                                }
                                TargetDomain::Kubernetes => {
                                    if let Ok(agent) = K8sAgent::from_yaml(&exp_config.target_config) {
                                        orchestrator.register_agent(Box::new(agent));
                                    }
                                }
                                TargetDomain::Server => {
                                    if let Ok(agent) = ServerAgent::from_yaml(&exp_config.target_config) {
                                        orchestrator.register_agent(Box::new(agent));
                                    }
                                }
                            }

                            tracing::info!(experiment = %exp_name, "Scheduled experiment starting");
                            match orchestrator.run_experiment(exp_config).await {
                                Ok(report) => {
                                    tracing::info!(experiment = %exp_name, report = %report, "Scheduled experiment completed");
                                }
                                Err(e) => {
                                    tracing::error!(experiment = %exp_name, error = %e, "Scheduled experiment failed");
                                }
                            }
                        });
                    }
                }

                last_check = now;
            }
            _ = shutdown_rx.changed() => {
                tracing::info!("Shutdown signal received, stopping scheduler");
                break;
            }
        }
    }

    // Wait for all running experiments to complete
    tracing::info!("Waiting for running experiments to finish...");
    let _ = semaphore
        .acquire_many(config.settings.max_concurrent as u32)
        .await;
    tracing::info!("All experiments completed, daemon exiting");

    // Clean up PID file
    if let Some(ref pid_path) = args.pid_file {
        let _ = std::fs::remove_file(pid_path);
    }

    Ok(())
}
