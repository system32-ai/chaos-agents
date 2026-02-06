use clap::Subcommand;

pub mod daemon;
pub mod list_skills;
pub mod plan;
pub mod run;
pub mod validate;

#[derive(Subcommand)]
pub enum Commands {
    /// Run a chaos experiment from a config file
    Run(run::RunArgs),
    /// Use an LLM to plan and orchestrate chaos experiments
    Plan(plan::PlanArgs),
    /// Start in daemon mode with scheduled experiments
    Daemon(daemon::DaemonArgs),
    /// List all available chaos skills
    ListSkills(list_skills::ListSkillsArgs),
    /// Validate a config file without executing
    Validate(validate::ValidateArgs),
}
