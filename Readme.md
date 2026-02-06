# Chaos Agents

**Autonomous chaos engineering, driven by intelligent agents.**

Chaos Agents deploys AI-powered agents into your infrastructure that autonomously discover resources, inject faults, observe the blast radius, and cleanly revert — all without human intervention. Instead of writing brittle test scripts, you declare *what* to stress and the agents figure out *how*.

## Why Chaos Agents?

Traditional chaos tools require you to hand-craft every failure scenario. Chaos Agents flips this: agents explore your systems, understand their topology, and generate chaos experiments on the fly. The result is deeper coverage, less maintenance, and failures you never thought to test for.

## Targets

### Databases
Agents connect to your database, introspect the schema, and unleash realistic workloads — high-volume inserts, concurrent updates, adversarial selects, and live configuration mutations. They measure the impact, then roll everything back to the exact prior state.

**Supported:** PostgreSQL, MySQL

### Kubernetes Clusters
Agents discover running workloads across namespaces and systematically degrade the cluster — killing pods, evicting nodes, injecting network partitions, corrupting DNS. Every action is reversible. Every action is logged.

### Servers
Agents SSH into target hosts and wreak controlled havoc — filling disks, killing processes, rotating permissions, saturating ports. When the experiment window closes, the agent restores the machine to its original state.

## Architecture

```
chaos-cli           CLI & daemon scheduler
    |
chaos-llm           LLM orchestration (Anthropic, OpenAI, Ollama) + MCP tools
    |
chaos-core          Orchestrator, agent traits, skill system, rollback engine
    |
  +--------+-----------+
  |        |           |
chaos-db  chaos-k8s  chaos-server
```

| Crate | Purpose |
|-------|---------|
| **chaos-core** | `Agent` and `Skill` traits, experiment orchestrator, LIFO rollback engine, event system, YAML config |
| **chaos-db** | Database agent. Schema discovery via `information_schema`, skills for insert/update/select load and config mutation |
| **chaos-k8s** | Kubernetes agent. Pod kill, node drain, network policy chaos, resource stress |
| **chaos-server** | Server agent. Auto-discovers running services via SSH, then targets disk, permissions, services, CPU, memory |
| **chaos-llm** | Multi-provider LLM layer (Anthropic, OpenAI, Ollama), tool system, MCP server support (stdio + SSE) |
| **chaos-cli** | The `chaos` binary. One-off runs, LLM-driven planning, daemon scheduling, skill listing, config validation |

## How It Works

1. **Discover** — The agent connects to the target and maps every resource: tables, pods, services, processes, filesystems.
2. **Plan** — The orchestrator (or an LLM) selects skills, parameterizes them, and builds an execution plan.
3. **Execute** — Skills run sequentially or in parallel, each returning a rollback handle.
4. **Observe** — Events stream to sinks in real time for monitoring and alerting.
5. **Rollback** — When the experiment window expires (or on failure), all actions revert in LIFO order. No residue.

## Installation

```bash
# From source
cargo install --path crates/chaos-cli

# Or build with make
make build
```

## Usage

### List available skills

```bash
chaos list-skills
chaos list-skills --target database
chaos list-skills --target kubernetes
chaos list-skills --target server
```

Output:

```
SKILL                     TARGET       DESCRIPTION
----------------------------------------------------------------------
db.insert_load            database     Bulk INSERT random rows into target tables
db.update_load            database     Randomly UPDATE existing rows in target tables
db.select_load            database     Generate heavy SELECT query load against target tables
db.config_change          database     ALTER database configuration parameters with rollback
k8s.pod_kill              kubernetes   Delete random pods matching label selector
k8s.node_drain            kubernetes   Cordon a node (mark unschedulable), rollback uncordons it
k8s.network_chaos         kubernetes   Apply deny-all NetworkPolicy to isolate pods
k8s.resource_stress       kubernetes   Deploy a stress-ng pod to consume cluster resources
server.disk_fill          server       Fill disk space with a large file, rollback removes it
server.permission_change  server       Change file permissions to disrupt services, rollback restores them
server.service_stop       server       Stop random running services, rollback restarts them
server.cpu_stress         server       Run stress-ng to load CPU, rollback kills the process
server.memory_stress      server       Run stress-ng to consume memory, rollback kills the process
```

### Run experiments from a config file

```bash
# Run a database chaos experiment
chaos run config/example-db.yaml

# Run a Kubernetes chaos experiment
chaos run config/example-k8s.yaml

# Run a server chaos experiment
chaos run config/example-server.yaml

# Dry-run (validate and discover only, no execution)
chaos run config/example-db.yaml --dry-run
```

### Validate a config without running it

```bash
chaos validate config/example-db.yaml
```

### LLM-driven chaos planning

Use an LLM to decide what chaos to create based on your infrastructure:

```bash
# Using Anthropic Claude (default)
export ANTHROPIC_API_KEY="sk-ant-..."
chaos plan "Test our PostgreSQL database resilience under heavy write load"

# Using OpenAI
export OPENAI_API_KEY="sk-..."
chaos plan "Kill random pods in the staging namespace" --provider openai

# Using Ollama (local)
chaos plan "Stress test the web servers" --provider ollama --model llama3.1

# Using a config file with MCP servers
chaos plan "Run chaos on the entire staging environment" --config config/example-llm.yaml
```

### Daemon mode (scheduled chaos)

Run experiments on a cron schedule:

```bash
chaos daemon config/daemon.yaml

# With a PID file for process management
chaos daemon config/daemon.yaml --pid-file /var/run/chaos.pid
```

## Configuration

### Experiment config

```yaml
experiments:
  - name: "postgres-load-test"
    target: database
    target_config:
      connection_url: "postgres://user:pass@localhost:5432/mydb"
      db_type: postgres
    skills:
      - skill_name: "db.insert_load"
        params:
          rows_per_table: 10000
          tables: ["users", "orders"]
      - skill_name: "db.config_change"
        params:
          changes:
            - param: "work_mem"
              value: "4MB"
    duration: "5m"
    parallel: false
```

### Kubernetes config

```yaml
experiments:
  - name: "k8s-pod-chaos"
    target: kubernetes
    target_config:
      namespace: "staging"
      label_selector: "app=web"
    skills:
      - skill_name: "k8s.pod_kill"
        params:
          namespace: "staging"
          label_selector: "app=web"
          count: 2
      - skill_name: "k8s.network_chaos"
        params:
          namespace: "staging"
          pod_selector:
            app: "web"
    duration: "5m"
```

### Server config

The server agent auto-discovers running services and targets chaos based on what it finds:

```yaml
experiments:
  - name: "server-chaos"
    target: server
    target_config:
      hosts:
        - host: "10.0.1.50"
          port: 22
          username: "chaos-agent"
          auth:
            type: key
            private_key_path: "~/.ssh/id_ed25519"
      discovery:
        enabled: true
        exclude_services: ["docker", "containerd"]
    skills:
      - skill_name: "server.service_stop"
        params:
          max_services: 2
      - skill_name: "server.disk_fill"
        params:
          size: "5GB"
          target_mount: "/tmp"
    duration: "10m"
    resource_filters:
      - "nginx.*"
      - "postgres.*"
```

### Daemon (scheduled) config

```yaml
settings:
  max_concurrent: 2

experiments:
  - experiment:
      name: "nightly-db-chaos"
      target: database
      target_config:
        connection_url: "postgres://chaos:pw@db:5432/staging"
        db_type: postgres
      skills:
        - skill_name: "db.insert_load"
          params:
            rows_per_table: 5000
      duration: "15m"
    schedule: "0 0 2 * * *"
    enabled: true
```

### LLM + MCP config

```yaml
llm:
  provider: anthropic
  api_key: "${ANTHROPIC_API_KEY}"
  model: "claude-sonnet-4-5-20250929"
  max_tokens: 4096

mcp_servers:
  - name: "prometheus-mcp"
    transport:
      type: stdio
      command: "npx"
      args: ["-y", "@modelcontextprotocol/server-prometheus"]
    env:
      PROMETHEUS_URL: "http://prometheus:9090"

max_turns: 10
```

## Rollback guarantees

Every skill captures the pre-mutation state before making changes. Rollback always happens in LIFO order:

| Skill | Action | Rollback |
|-------|--------|----------|
| `db.insert_load` | INSERT rows | DELETE by stored IDs |
| `db.config_change` | ALTER SYSTEM SET | Restore original value |
| `k8s.pod_kill` | Delete pod | Verify replacement pod is running |
| `k8s.node_drain` | Cordon node | Uncordon node |
| `k8s.network_chaos` | Create deny-all NetworkPolicy | Delete the policy |
| `k8s.resource_stress` | Deploy stress-ng pod | Delete the pod |
| `server.disk_fill` | Allocate large file | Remove the file |
| `server.permission_change` | chmod to 000 | Restore original permissions |
| `server.service_stop` | systemctl stop | systemctl start |
| `server.cpu_stress` | Run stress-ng CPU | Kill the process |
| `server.memory_stress` | Run stress-ng memory | Kill the process |

If the process crashes mid-experiment, the rollback log is serializable and can be replayed on restart.

## Roadmap

- **Adaptive chaos** — Agents that learn from past experiments and automatically escalate intensity toward failure boundaries.
- **Multi-target experiments** — Coordinated chaos across database + cluster + server in a single experiment.
- **Observability integrations** — Stream events to Prometheus, Grafana, Datadog, and PagerDuty.
- **Steady-state assertions** — Define success criteria and let the agent validate system resilience automatically.
- **Cloud-native targets** — AWS, GCP, and Azure resource-level fault injection (Lambda throttling, S3 latency, IAM revocation).
- **Distributed agent mesh** — Agents coordinating across regions to simulate real-world cascading failures.

## License

MIT
