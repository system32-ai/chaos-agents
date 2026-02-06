use std::path::PathBuf;

use clap::Args;

use chaos_core::config::ChaosConfig;
use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_k8s::agent::K8sAgent;
use chaos_server::agent::ServerAgent;

#[derive(Args)]
pub struct ValidateArgs {
    /// Path to config file to validate
    pub config: PathBuf,
}

pub async fn execute(args: ValidateArgs) -> anyhow::Result<()> {
    println!("Validating {}...", args.config.display());

    let config = ChaosConfig::from_file(&args.config)?;
    println!("  YAML parsing: OK");
    println!("  Experiments found: {}", config.experiments.len());

    let mut errors = Vec::new();

    for (i, experiment) in config.experiments.iter().enumerate() {
        println!("\n  Experiment #{}: '{}'", i + 1, experiment.name);
        println!("    Target: {}", experiment.target);
        println!("    Duration: {:?}", experiment.duration);
        println!("    Skills: {}", experiment.skills.len());

        // Validate target config can be parsed
        let agent_result: Result<Box<dyn chaos_core::agent::Agent>, _> = match experiment.target {
            TargetDomain::Database => DbAgent::from_yaml(&experiment.target_config)
                .map(|a| Box::new(a) as Box<dyn chaos_core::agent::Agent>),
            TargetDomain::Kubernetes => K8sAgent::from_yaml(&experiment.target_config)
                .map(|a| Box::new(a) as Box<dyn chaos_core::agent::Agent>),
            TargetDomain::Server => ServerAgent::from_yaml(&experiment.target_config)
                .map(|a| Box::new(a) as Box<dyn chaos_core::agent::Agent>),
        };

        match agent_result {
            Ok(agent) => {
                println!("    Target config: OK");

                // Validate each skill exists and params are valid
                for invocation in &experiment.skills {
                    match agent.skill_by_name(&invocation.skill_name) {
                        Some(skill) => {
                            match skill.validate_params(&invocation.params) {
                                Ok(()) => {
                                    println!("    Skill '{}': OK", invocation.skill_name);
                                }
                                Err(e) => {
                                    let msg = format!(
                                        "Experiment '{}', skill '{}': invalid params: {e}",
                                        experiment.name, invocation.skill_name
                                    );
                                    println!("    Skill '{}': INVALID - {e}", invocation.skill_name);
                                    errors.push(msg);
                                }
                            }
                        }
                        None => {
                            let msg = format!(
                                "Experiment '{}': unknown skill '{}'",
                                experiment.name, invocation.skill_name
                            );
                            println!("    Skill '{}': NOT FOUND", invocation.skill_name);
                            errors.push(msg);
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Experiment '{}': invalid target config: {e}", experiment.name);
                println!("    Target config: INVALID - {e}");
                errors.push(msg);
            }
        }
    }

    println!();
    if errors.is_empty() {
        println!("Validation PASSED");
    } else {
        println!("Validation FAILED with {} error(s):", errors.len());
        for err in &errors {
            eprintln!("  - {err}");
        }
        std::process::exit(1);
    }

    Ok(())
}
