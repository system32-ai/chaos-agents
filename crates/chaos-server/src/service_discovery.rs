use chaos_core::discovery::{ServerResource, ServerResourceType};

use crate::ssh::SshSession;

/// Services that must never be targeted for chaos.
const EXCLUDED_SERVICES: &[&str] = &[
    "sshd",
    "ssh",
    "systemd",
    "dbus",
    "dbus-broker",
    "NetworkManager",
    "network",
    "firewalld",
    "iptables",
    "ufw",
    "chaos-agent",
];

pub struct ServiceDiscoverer;

impl ServiceDiscoverer {
    /// Discover running services, listening ports, and filesystems on a remote host.
    pub async fn discover(
        ssh: &SshSession,
        user_excludes: &[String],
    ) -> anyhow::Result<Vec<ServerResource>> {
        let mut resources = Vec::new();

        // Step 1: Discover systemd services
        let services = Self::discover_services(ssh, user_excludes).await?;
        resources.extend(services);

        // Step 2: Discover listening ports
        let ports = Self::discover_ports(ssh).await?;
        resources.extend(ports);

        // Step 3: Discover mounted filesystems
        let filesystems = Self::discover_filesystems(ssh).await?;
        resources.extend(filesystems);

        Ok(resources)
    }

    async fn discover_services(
        ssh: &SshSession,
        user_excludes: &[String],
    ) -> anyhow::Result<Vec<ServerResource>> {
        let (exit_code, stdout, _stderr) = ssh
            .exec("systemctl list-units --type=service --state=running --no-legend --plain 2>/dev/null || true")
            .await?;

        if exit_code != 0 && stdout.is_empty() {
            return Ok(Vec::new());
        }

        let mut services = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            let service_name = parts[0].trim_end_matches(".service");

            // Check exclusion lists
            if EXCLUDED_SERVICES
                .iter()
                .any(|&e| service_name.contains(e))
            {
                continue;
            }
            if user_excludes
                .iter()
                .any(|e| service_name.contains(e.as_str()))
            {
                continue;
            }

            let description = if parts.len() > 4 {
                parts[4..].join(" ")
            } else {
                String::new()
            };

            services.push(ServerResource {
                host: ssh.host.clone(),
                resource_type: ServerResourceType::RunningService,
                name: service_name.to_string(),
                details: serde_yaml::to_value(serde_json::json!({
                    "unit": parts[0],
                    "load": parts.get(1).unwrap_or(&""),
                    "active": parts.get(2).unwrap_or(&""),
                    "sub": parts.get(3).unwrap_or(&""),
                    "description": description,
                }))
                .unwrap_or(serde_yaml::Value::Null),
            });
        }

        tracing::info!(
            host = %ssh.host,
            count = services.len(),
            "Discovered running services"
        );

        Ok(services)
    }

    async fn discover_ports(ssh: &SshSession) -> anyhow::Result<Vec<ServerResource>> {
        let (_, stdout, _) = ssh
            .exec("ss -tlnp 2>/dev/null || netstat -tlnp 2>/dev/null || true")
            .await?;

        let mut ports = Vec::new();

        for line in stdout.lines().skip(1) {
            // Skip header
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            // Parse local address:port
            let local_addr = parts.get(3).unwrap_or(&"");
            if let Some(port_str) = local_addr.rsplit(':').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    // Extract process name if available
                    let process = parts
                        .iter()
                        .find(|p| p.contains("users:"))
                        .map(|p| {
                            p.trim_start_matches("users:((\"")
                                .split('"')
                                .next()
                                .unwrap_or("")
                                .to_string()
                        })
                        .unwrap_or_default();

                    ports.push(ServerResource {
                        host: ssh.host.clone(),
                        resource_type: ServerResourceType::ListeningPort,
                        name: format!("port-{port}"),
                        details: serde_yaml::to_value(serde_json::json!({
                            "port": port,
                            "address": local_addr,
                            "process": process,
                        }))
                        .unwrap_or(serde_yaml::Value::Null),
                    });
                }
            }
        }

        tracing::info!(host = %ssh.host, count = ports.len(), "Discovered listening ports");

        Ok(ports)
    }

    async fn discover_filesystems(ssh: &SshSession) -> anyhow::Result<Vec<ServerResource>> {
        let (_, stdout, _) = ssh
            .exec("df -h --output=target,pcent,avail,fstype 2>/dev/null || df -h 2>/dev/null || true")
            .await?;

        let mut filesystems = Vec::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            let mount = parts[0];
            let usage_pct = parts[1].trim_end_matches('%');
            let available = parts[2];
            let fs_type = parts[3];

            // Skip virtual filesystems
            if ["tmpfs", "devtmpfs", "squashfs", "overlay", "proc", "sysfs", "devpts"]
                .contains(&fs_type)
            {
                continue;
            }

            filesystems.push(ServerResource {
                host: ssh.host.clone(),
                resource_type: ServerResourceType::MountedFilesystem,
                name: mount.to_string(),
                details: serde_yaml::to_value(serde_json::json!({
                    "mount": mount,
                    "usage_percent": usage_pct,
                    "available": available,
                    "fs_type": fs_type,
                }))
                .unwrap_or(serde_yaml::Value::Null),
            });
        }

        tracing::info!(
            host = %ssh.host,
            count = filesystems.len(),
            "Discovered filesystems"
        );

        Ok(filesystems)
    }
}
