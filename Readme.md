# Chaos Agents

Chaos engineering tool that uses agents to break your infrastructure on purpose, then clean up after itself.

You tell it what to target (a database, a k8s cluster, some servers), pick the skills you want to run, and it handles discovery, fault injection, and rollback. You can also point an LLM at your infra and let it decide what to break.

## What it does

**Databases** (PostgreSQL, MySQL) — Connects to your DB, looks at the schema, and hammers it with inserts, updates, heavy reads, or config changes. Rolls back everything when done.

**Kubernetes** — Finds workloads in your cluster and starts killing pods, cordoning nodes, dropping network policies, or deploying resource hogs. Cleans up on exit.

**Servers** — SSHes into hosts, discovers what's running (services, ports, filesystems), and goes after them: fills disks, stops services, changes permissions, spikes CPU/memory. Restores original state after.

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

| Crate | What it does |
|-------|-------------|
| **chaos-core** | `Agent` and `Skill` traits, experiment orchestrator, LIFO rollback engine, event system, YAML config |
| **chaos-db** | Database agent — schema discovery via `information_schema`, insert/update/select load, config mutation |
| **chaos-k8s** | Kubernetes agent — pod kill, node drain, network policy injection, resource stress |
| **chaos-server** | Server agent — auto-discovers running services via SSH, targets disk/permissions/services/CPU/memory |
| **chaos-llm** | LLM providers (Anthropic, OpenAI, Ollama), tool system, MCP server support (stdio + SSE) |
| **chaos-cli** | The `chaos` binary — run experiments, LLM planning, daemon scheduling, skill listing, config validation |

## How it works

1. **Discover** — Agent connects to the target and figures out what's there (tables, pods, services, filesystems, etc.)
2. **Plan** — The orchestrator (or an LLM) picks skills and sets parameters
3. **Execute** — Skills run and each one saves what it needs for rollback
4. **Observe** — Events get emitted in real time
5. **Rollback** — When the duration expires (or something fails), everything reverts in LIFO order

## Install

### Quick install (latest release)

```bash
curl -fsSL https://raw.githubusercontent.com/system32-ai/chaos-agents/master/install.sh | bash
```

You can also set a specific version or install directory:

```bash
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/system32-ai/chaos-agents/master/install.sh | bash

# custom install location
INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/system32-ai/chaos-agents/master/install.sh | bash
```

### Build from source

```bash
cargo install --path crates/chaos-cli

# or
make build
```

## Usage

### List skills

```bash
chaos list-skills
chaos list-skills --target database
chaos list-skills --target kubernetes
chaos list-skills --target server
```

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

### Run experiments

```bash
chaos run config/example-db.yaml
chaos run config/example-k8s.yaml
chaos run config/example-server.yaml

# dry-run — validates and discovers but doesn't execute anything
chaos run config/example-db.yaml --dry-run
```

### Validate config

```bash
chaos validate config/example-db.yaml
```

### LLM planning

Let an LLM look at your setup and decide what chaos to run:

```bash
# Anthropic (default)
export ANTHROPIC_API_KEY="sk-ant-..."
chaos plan "Test our PostgreSQL database resilience under heavy write load"

# OpenAI
export OPENAI_API_KEY="sk-..."
chaos plan "Kill random pods in the staging namespace" --provider openai

# Ollama (local)
chaos plan "Stress test the web servers" --provider ollama --model llama3.1

# With MCP servers for extra context
chaos plan "Run chaos on the entire staging environment" --config config/example-llm.yaml
```

### Daemon mode

Run experiments on a cron schedule:

```bash
chaos daemon config/daemon.yaml

# with a PID file
chaos daemon config/daemon.yaml --pid-file /var/run/chaos.pid
```

## Configuration

### Database experiment

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

### Kubernetes experiment

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

### Server experiment

The server agent auto-discovers running services and picks targets based on what it finds:

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

### Daemon config

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

## Rollback

Every skill saves the original state before doing anything. Rollback happens in LIFO order — last thing changed gets reverted first.

| Skill | What it does | Rollback |
|-------|-------------|----------|
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

If the process crashes mid-experiment, the rollback log is serializable so it can be replayed on restart.

## Roadmap

- Adaptive chaos — agents that learn from past runs and escalate intensity on their own
- Multi-target experiments — coordinated chaos across DB + k8s + server in one go
- Observability integrations — Prometheus, Grafana, Datadog, PagerDuty
- Steady-state assertions — define what "healthy" looks like and let the agent check
- Cloud targets — AWS, GCP, Azure fault injection (Lambda throttling, S3 latency, IAM revocation)
- Distributed agent mesh — agents across regions for cascading failure scenarios

## License

MIT
