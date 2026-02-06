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
chaos-cli          CLI & daemon scheduler
    |
chaos-core         Orchestrator, agent traits, skill system, rollback engine
    |
  +--------+-----------+
  |        |           |
chaos-db  chaos-k8s  chaos-server
```

- **chaos-core** — The brain. Defines the `Agent` and `Skill` traits, the experiment orchestrator, LIFO rollback engine, event system, and YAML-driven configuration.
- **chaos-db** — Database agent. Schema discovery via `information_schema`, connection pooling, and skills for insert/update/select load and config mutation.
- **chaos-k8s** — Kubernetes agent. Pod, node, and network-level chaos via the Kubernetes API.
- **chaos-server** — Server agent. Host-level chaos over SSH — processes, filesystems, networking.
- **chaos-cli** — The `chaos` binary. Run one-off experiments or schedule recurring chaos via cron.

## How It Works

1. **Discover** — The agent connects to the target and maps every resource: tables, pods, services, processes.
2. **Plan** — The orchestrator selects skills, parameterizes them, and builds an execution plan.
3. **Execute** — Skills run sequentially or in parallel, each returning a rollback handle.
4. **Observe** — Events stream to sinks in real time for monitoring and alerting.
5. **Rollback** — When the experiment window expires (or on failure), all actions revert in LIFO order. No residue.

## Configuration

Experiments are defined in YAML:

```yaml
experiments:
  - name: stress-checkout-db
    target: database
    skills:
      - name: insert_load
        params:
          tables: ["orders", "payments"]
          rows_per_second: 5000
        count: 3
      - name: config_change
        params:
          setting: work_mem
          value: "8MB"
    duration: 5m
    parallel: true
```

Schedule recurring chaos with cron:

```yaml
daemon:
  settings:
    max_concurrent: 2
    health_bind: "0.0.0.0:9090"
  schedules:
    - experiment: stress-checkout-db
      cron: "0 3 * * *"
```

## Roadmap

- **Adaptive chaos** — Agents that learn from past experiments and automatically escalate intensity toward failure boundaries.
- **Multi-target experiments** — Coordinated chaos across database + cluster + server in a single experiment.
- **Observability integrations** — Stream events to Prometheus, Grafana, Datadog, and PagerDuty.
- **Steady-state assertions** — Define success criteria and let the agent validate system resilience automatically.
- **Cloud-native targets** — AWS, GCP, and Azure resource-level fault injection (Lambda throttling, S3 latency, IAM revocation).
- **Distributed agent mesh** — Agents coordinating across regions to simulate real-world cascading failures.
- **Natural language experiments** — Describe chaos in plain English; the agent compiles it into a runnable plan.

## Getting Started

```bash
# Build
cargo build --release

# Run a single experiment
./target/release/chaos run --config experiments.yaml

# Start the daemon
./target/release/chaos daemon --config daemon.yaml
```

## License

MIT
