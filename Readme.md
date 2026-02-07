# Chaos Agents

Chaos engineering tool that uses agents to break your infrastructure on purpose, then clean up after itself.

You tell it what to target (a database, a k8s cluster, some servers), pick the skills you want to run, and it handles discovery, fault injection, and rollback. You can also point an LLM at your infra and let it decide what to break.

**Databases** (PostgreSQL, MySQL, CockroachDB, YugabyteDB, MongoDB) — Connects to your DB, discovers the schema (or collections for MongoDB), and hammers it with inserts, updates, heavy reads, or config changes. Rolls back everything when done.

**Kubernetes** — Finds workloads in your cluster and starts killing pods, cordoning nodes, dropping network policies, or deploying resource hogs. Cleans up on exit.

**Servers** — SSHes into hosts, discovers what's running (services, ports, filesystems), and goes after them: fills disks, stops services, changes permissions, spikes CPU/memory. Restores original state after.

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
mongo.insert_load         database     Bulk INSERT random documents into MongoDB collections
mongo.update_load         database     Randomly UPDATE existing documents in MongoDB collections
mongo.find_load           database     Generate heavy read (find) query load against MongoDB collections
mongo.index_drop          database     Drop secondary indexes from MongoDB collections
mongo.profiling_change    database     Change MongoDB profiling level to add overhead
mongo.connection_pool_stress database  Open many MongoDB connections to exhaust limits
crdb.zone_config_change   database     Change CockroachDB zone config (replication, GC TTL)
ysql.follower_reads       database     Toggle YugabyteDB follower reads for eventual consistency
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

Let an LLM look at your setup and decide what chaos to run. The provider is auto-detected from your API key environment variables:

```bash
# Anthropic — auto-detected from ANTHROPIC_API_KEY
export ANTHROPIC_API_KEY="sk-ant-..."
chaos plan "Test our PostgreSQL database resilience under heavy write load"

# OpenAI — auto-detected from OPENAI_API_KEY
export OPENAI_API_KEY="sk-..."
chaos plan "Kill random pods in the staging namespace"

# Ollama (local) — used as fallback when no API key is set
chaos plan "Stress test the web servers" --model llama3.1

# Explicit provider override
chaos plan "Break the database" --provider openai

# With MCP servers for extra context
chaos plan "Run chaos on the entire staging environment" --config config/example-llm.yaml
```

### Agent mode

Plan and execute in one step — the LLM generates experiments, you review, and approve:

```bash
# Plan and run interactively
chaos agent "Test our PostgreSQL database resilience under heavy write load"

# Target CockroachDB or YugabyteDB — auto-detected from prompt keywords
chaos agent "Test cockroachdb resilience at postgres://root@localhost:26257/mydb"

# MongoDB — auto-detected from mongodb:// URL
chaos agent "Load test mongodb://localhost:27017 collections"

# Preview the generated config without executing
chaos agent "Kill random pods in staging" --dry-run

# Auto-approve (skip confirmation)
chaos agent "Stress test the web servers" -y

# Save the generated config to a file and run
chaos agent "Fill disk on 10.0.1.50" --save plan.yaml
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

### CockroachDB experiment

CockroachDB and YugabyteDB are PostgreSQL wire-compatible, so they use `postgres://` connection URLs. The SQL skills (`db.insert_load`, `db.select_load`, `db.update_load`) work as-is. The `db.config_change` skill uses CockroachDB's `SET CLUSTER SETTING` syntax automatically.

```yaml
experiments:
  - name: "cockroachdb-resilience"
    target: database
    target_config:
      connection_url: "postgres://root@localhost:26257/mydb"
      db_type: cockroach_db
    skills:
      - skill_name: "db.insert_load"
        params:
          rows_per_table: 5000
      - skill_name: "crdb.zone_config_change"
        params:
          target: "DATABASE mydb"
          changes:
            - param: "num_replicas"
              value: "1"
            - param: "gc.ttlseconds"
              value: "600"
    duration: "5m"
```

### YugabyteDB experiment

```yaml
experiments:
  - name: "yugabyte-consistency-test"
    target: database
    target_config:
      connection_url: "postgres://yugabyte@localhost:5433/mydb"
      db_type: yugabyte_db
    skills:
      - skill_name: "db.insert_load"
        params:
          rows_per_table: 5000
      - skill_name: "ysql.follower_reads"
        params:
          enable: true
          staleness: "60000ms"
    duration: "5m"
```

### MongoDB experiment

```yaml
experiments:
  - name: "mongodb-load-test"
    target: database
    target_config:
      connection_url: "mongodb://localhost:27017"
      db_type: mongo_d_b
      databases: ["myapp"]
    skills:
      - skill_name: "mongo.insert_load"
        params:
          database: "myapp"
          docs_per_collection: 5000
      - skill_name: "mongo.update_load"
        params:
          database: "myapp"
          docs: 200
      - skill_name: "mongo.find_load"
        params:
          database: "myapp"
          query_count: 1000
      - skill_name: "mongo.index_drop"
        params:
          database: "myapp"
          max_per_collection: 2
      - skill_name: "mongo.profiling_change"
        params:
          database: "myapp"
          level: 2
    duration: "5m"
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
| `db.update_load` | UPDATE rows | Restore original values |
| `db.select_load` | Heavy SELECT queries | No-op (read-only) |
| `db.config_change` | ALTER SYSTEM SET / SET CLUSTER SETTING | Restore original value |
| `mongo.insert_load` | INSERT documents | DELETE by stored ObjectIds |
| `mongo.update_load` | UPDATE documents | Replace with original documents |
| `mongo.find_load` | Heavy find/aggregate queries | No-op (read-only) |
| `mongo.index_drop` | Drop secondary indexes | Recreate indexes with original key/options |
| `mongo.profiling_change` | Set profiling level to 2 (all ops) | Restore original profiling level |
| `mongo.connection_pool_stress` | Open many connections | Connections drain on process exit |
| `crdb.zone_config_change` | ALTER zone config (replication, GC) | Re-apply original zone config |
| `ysql.follower_reads` | Enable follower reads + staleness | Restore original follower read settings |
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

## Community

Join us on [Discord](https://discord.com/channels/1469489696336908361/1469489765219958968) for questions, feedback, and discussion.

## License

MIT
