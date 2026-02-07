# Chaos Agents v0.3.0

Chaos engineering tool that uses LLM-driven agents to break your infrastructure on purpose, then clean up after itself.

## Highlights

### Terminal UI

Run `chaos` with no arguments to launch an interactive TUI with a setup wizard, live experiment dashboard, and real-time progress tracking.

```bash
chaos
```

### Agent Mode (`chaos agent`)

Plan and execute chaos experiments in one step. An LLM analyzes your infrastructure, discovers resources, generates experiments, and runs them after your approval.

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
chaos agent "Test our PostgreSQL resilience under heavy write load"
```

- Real-time progress output showing LLM thinking, tool calls, and discovery results
- Interactive y/N confirmation before execution
- `--dry-run` to preview the generated config without running
- `--save plan.yaml` to export the generated experiment config
- `-y` to skip confirmation and auto-approve
- `--max-turns` to control LLM planning depth

### Auto-detected LLM Provider

No need to pass `--provider` anymore. The CLI auto-detects based on which API key environment variable is set:

- `ANTHROPIC_API_KEY` -> Anthropic
- `OPENAI_API_KEY` -> OpenAI
- Neither -> Ollama (local, no key needed)

### Graceful Ctrl+C Cancellation

Press Ctrl+C during a running experiment to cancel gracefully. Remaining skills are skipped and the soak period is interrupted, but **rollback always runs** to restore your infrastructure to its original state.

## New Database Targets

### MongoDB

Full MongoDB chaos agent with 6 new skills:

| Skill | Description |
|-------|------------|
| `db.mongo_insert_load` | Bulk insert random documents into collections |
| `db.mongo_update_load` | Randomly update existing documents |
| `db.mongo_find_load` | Generate heavy read query load |
| `db.mongo_index_drop` | Drop indexes to degrade query performance |
| `db.mongo_profiling_change` | Alter database profiling level |
| `db.mongo_connection_stress` | Exhaust connection pool resources |

### CockroachDB

New CockroachDB-specific skill:

| Skill | Description |
|-------|------------|
| `db.crdb_zone_config` | Modify CockroachDB zone configuration with rollback |

### YugabyteDB

New YugabyteDB-specific skill:

| Skill | Description |
|-------|------------|
| `db.ysql_follower_reads` | Toggle YugabyteDB follower reads settings |

## Experiment Reports

Experiments now produce structured reports with:

- Discovered resource summary
- Per-skill execution records (duration, success/failure)
- Rollback step records with timing
- Overall experiment status and duration

## New Infrastructure

### Install Script

One-line install for all supported platforms:

```bash
curl -fsSL https://raw.githubusercontent.com/system32-ai/chaos-agents/master/install.sh | bash
```

### Release Tooling

- `release.sh` — builds Linux targets in Docker, macOS targets natively, creates GitHub release
- `Dockerfile.release` — multi-target Linux build container with cross-compilation support

## Other Improvements

- Live resource discovery during LLM planning (connects to actual targets instead of returning stubs)
- Planner auto-injects `target_config` into `run_experiment` calls from prior `discover_resources` results
- Fallback `target_config` extraction from natural language prompts (parses connection URLs)
- Planner event system (`PlannerEvent`) for TUI consumption
- Shared execution helpers extracted to `execution.rs` module

## Supported Platforms

| Platform | Architecture |
|----------|-------------|
| Linux | x86_64 (glibc, musl) |
| Linux | aarch64 (glibc, musl) |
| macOS | x86_64 (Intel) |
| macOS | aarch64 (Apple Silicon) |

## Install

```bash
# Quick install
curl -fsSL https://raw.githubusercontent.com/system32-ai/chaos-agents/master/install.sh | bash

# Build from source
cargo install --path crates/chaos-cli
```
