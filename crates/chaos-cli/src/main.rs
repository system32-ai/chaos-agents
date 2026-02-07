use clap::Parser;

mod commands;
pub mod execution;

#[derive(Parser)]
#[command(
    name = "chaos",
    about = "Chaos Agents - controlled chaos engineering for databases, Kubernetes, and servers",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<commands::Commands>,

    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            // No subcommand: launch TUI
            chaos_tui::launch_tui().await
        }
        Some(command) => {
            let filter = match cli.verbose {
                0 => "info",
                1 => "debug",
                _ => "trace",
            };
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .init();

            match command {
                commands::Commands::Run(args) => commands::run::execute(args).await,
                commands::Commands::Plan(args) => commands::plan::execute(args).await,
                commands::Commands::Agent(args) => commands::agent::execute(args).await,
                commands::Commands::Daemon(args) => commands::daemon::execute(args).await,
                commands::Commands::ListSkills(args) => {
                    commands::list_skills::execute(args).await
                }
                commands::Commands::Validate(args) => commands::validate::execute(args).await,
            }
        }
    }
}
