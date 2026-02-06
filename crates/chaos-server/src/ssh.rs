use async_ssh2_tokio::client::{AuthMethod, Client, ServerCheckMethod};

use crate::config::HostConfig;

pub struct SshSession {
    client: Client,
    pub host: String,
}

impl SshSession {
    pub async fn connect(config: &HostConfig) -> anyhow::Result<Self> {
        let auth = match &config.auth {
            crate::config::AuthConfig::Key { private_key_path } => {
                let expanded = shellexpand::tilde(private_key_path).to_string();
                let key = std::fs::read_to_string(&expanded)?;
                AuthMethod::with_key(&key, None)
            }
            crate::config::AuthConfig::Password { password } => {
                AuthMethod::with_password(password)
            }
        };

        let client = Client::connect(
            (config.host.as_str(), config.port),
            &config.username,
            auth,
            ServerCheckMethod::NoCheck,
        )
        .await?;

        Ok(Self {
            client,
            host: config.host.clone(),
        })
    }

    /// Execute a remote command and return (exit_code, stdout, stderr).
    pub async fn exec(&self, command: &str) -> anyhow::Result<(i32, String, String)> {
        let result = self.client.execute(command).await?;
        Ok((
            result.exit_status as i32,
            result.stdout,
            result.stderr,
        ))
    }
}
