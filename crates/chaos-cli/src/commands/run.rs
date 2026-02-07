use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;

use chaos_core::config::ChaosConfig;
use chaos_core::event::TracingEventSink;
use chaos_core::orchestrator::Orchestrator;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_db::mongo_agent::MongoAgent;
use chaos_k8s::agent::K8sAgent;
use chaos_server::agent::ServerAgent;

#[derive(Args)]
pub struct RunArgs {
    /// Path to the experiment YAML config file
    pub config: PathBuf,
    /// Dry-run mode: discover and validate but don't execute
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn execute(args: RunArgs) -> anyhow::Result<()> {
    let config = ChaosConfig::from_file(&args.config)?;

    tracing::info!(
        experiments = config.experiments.len(),
        "Loaded configuration"
    );

    let mut orchestrator = Orchestrator::new();
    orchestrator.add_event_sink(Arc::new(TracingEventSink));

    for experiment in &config.experiments {
        // Register the appropriate agent
        match experiment.target {
            TargetDomain::Database => {
                let is_mongo = experiment
                    .target_config
                    .get("db_type")
                    .and_then(|v| v.as_str())
                    .map_or(false, |t| t == "mongo_d_b" || t == "mongodb" || t == "mongo");
                if is_mongo {
                    let agent = MongoAgent::from_yaml(&experiment.target_config)?;
                    orchestrator.register_agent(Box::new(agent));
                } else {
                    let agent = DbAgent::from_yaml(&experiment.target_config)?;
                    orchestrator.register_agent(Box::new(agent));
                }
            }
            TargetDomain::Kubernetes => {
                let agent = K8sAgent::from_yaml(&experiment.target_config)?;
                orchestrator.register_agent(Box::new(agent));
            }
            TargetDomain::Server => {
                let agent = ServerAgent::from_yaml(&experiment.target_config)?;
                orchestrator.register_agent(Box::new(agent));
            }
        }
    }

    if args.dry_run {
        tracing::info!("Dry-run mode: validating configuration only");
        for experiment in &config.experiments {
            tracing::info!(
                name = %experiment.name,
                target = %experiment.target,
                skills = experiment.skills.len(),
                duration = ?experiment.duration,
                "Experiment validated"
            );
        }
        println!("Configuration is valid.");
        return Ok(());
    }

    for experiment in config.experiments {
        tracing::info!(name = %experiment.name, "Starting experiment");
        match orchestrator.run_experiment(experiment.clone()).await {
            Ok(report) => {
                println!("{report}");
            }
            Err(e) => {
                eprintln!("Experiment '{}' failed: {e}", experiment.name);
            }
        }
    }

    Ok(())
}
