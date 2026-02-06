use kube::Client;

use crate::config::K8sTargetConfig;

pub async fn create_client(config: &K8sTargetConfig) -> anyhow::Result<Client> {
    let client = if let Some(ref path) = config.kubeconfig {
        let kubeconfig = kube::config::Kubeconfig::read_from(path)?;
        let kube_config = kube::Config::from_custom_kubeconfig(
            kubeconfig,
            &kube::config::KubeConfigOptions::default(),
        )
        .await?;
        Client::try_from(kube_config)?
    } else {
        Client::try_default().await?
    };
    Ok(client)
}
