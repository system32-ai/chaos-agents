use clap::Args;

use chaos_core::skill::TargetDomain;
use chaos_db::agent::DbAgent;
use chaos_db::config::{DbTargetConfig, DbType};
use chaos_k8s::agent::K8sAgent;
use chaos_k8s::config::K8sTargetConfig;
use chaos_server::agent::ServerAgent;
use chaos_server::config::ServerTargetConfig;

#[derive(Args)]
pub struct ListSkillsArgs {
    /// Filter by target domain (database, kubernetes, server)
    #[arg(long)]
    pub target: Option<String>,
}

pub async fn execute(args: ListSkillsArgs) -> anyhow::Result<()> {
    let filter: Option<TargetDomain> = args.target.as_deref().map(|t| match t {
        "database" | "db" => TargetDomain::Database,
        "kubernetes" | "k8s" => TargetDomain::Kubernetes,
        "server" | "srv" => TargetDomain::Server,
        _ => TargetDomain::Database, // fallback
    });

    // Create dummy agents to extract skill descriptors
    let db_agent = DbAgent::new(DbTargetConfig {
        connection_url: String::new(),
        db_type: DbType::Postgres,
        schemas: Vec::new(),
    });

    let k8s_agent = K8sAgent::new(K8sTargetConfig {
        kubeconfig: None,
        namespace: "default".into(),
        label_selector: None,
    });

    let server_agent = ServerAgent::new(ServerTargetConfig {
        hosts: Vec::new(),
        discovery: Default::default(),
    });

    println!("{:<25} {:<12} {}", "SKILL", "TARGET", "DESCRIPTION");
    println!("{}", "-".repeat(70));

    let agents: Vec<Box<dyn chaos_core::agent::Agent>> = vec![
        Box::new(db_agent),
        Box::new(k8s_agent),
        Box::new(server_agent),
    ];

    for agent in &agents {
        for skill in agent.skills() {
            let desc = skill.descriptor();
            if let Some(ref f) = filter {
                if &desc.target != f {
                    continue;
                }
            }
            println!("{:<25} {:<12} {}", desc.name, desc.target, desc.description);
        }
    }

    Ok(())
}
