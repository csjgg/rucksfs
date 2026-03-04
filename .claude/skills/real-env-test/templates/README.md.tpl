# {{PROJECT_NAME}} Automated Deployment Testing

Taskfile-based automated testing framework for {{PROJECT_NAME}}. Handles the
full lifecycle: **build → upload → install → configure → benchmark → collect
results** on a remote test host via SSH.

> **Agent-First Design**: All tasks are designed for AI agent execution — short-lived
> SSH calls, structured output, clear exit codes, and async benchmark support.

## Prerequisites

| Tool | Purpose |
|------|---------|
| [Task](https://taskfile.dev/installation/) | Task runner (v3+) |
| [yq](https://github.com/mikefarah/yq) | YAML parser (optional, falls back to grep/awk) |
| {{BUILD_TOOL}} | {{BUILD_TOOL_PURPOSE}} |
| SSH key access | Passwordless SSH to remote host |

The **remote host** must have:

- {{REMOTE_PREREQUISITES}}

## Quick Start

```bash
cd testing
cp env.example.yml env.yml
vim env.yml          # fill in host, SSH key, etc.
task test            # Full automated test (sync, blocks until done)
```

---

## Configuration

Copy `env.example.yml` to `env.yml` and edit:

| Section | Key Fields | Description |
|---------|------------|-------------|
| `remote` | `host`, `user`, `ssh_key`, `port` | SSH connection to the test machine |
| `bastion` | `enabled`, `host`, `user`, `ssh_key`, `port` | Jump host config (optional) |
| {{CONFIG_SECTIONS}} |

### Key Benchmark Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `benchmark.rate` | `1` | Target operations per second |
| `benchmark.duration` | `60` | Duration in seconds |
| {{BENCHMARK_PARAMS}} |

### Bastion / Jump Host

If the remote host is behind a bastion, enable it in `env.yml`:

```yaml
bastion:
  enabled: true
  host: "<bastion-ip>"
  user: "root"
  ssh_key: "~/.ssh/id_rsa"
  port: 22
```

All SSH/SCP commands automatically use `ssh -J` (ProxyJump).

---

## Available Tasks

List all tasks: `task --list`

### Composite Workflows

| Task | Mode | Description |
|------|:----:|-------------|
| `test` | sync | Full flow: validate → build → deploy → check → restart → bench → collect |
| `test:agent` | **async** | Same but launches benchmark in background, returns immediately |
| `quick-bench` | sync | Re-run: clean → restart → bench → collect (skip build/deploy) |
| `quick-bench:agent` | **async** | Same but launches benchmark in background |
| `clean-bench:agent` | **async** | Deep-clean + rotate dirs + reboot → configure → bench |

### Build Tasks

| Task | Description |
|------|-------------|
| `build` | Build all binaries |
| {{BUILD_TASK_ROWS}} |

### Deploy Tasks

| Task | Description |
|------|-------------|
| `deploy` | Full deploy: upload + install + configure |
| `deploy:upload` | Upload binaries, scripts, and dataset to remote |
| `deploy:install` | Install components on remote host |
| `deploy:configure` | Configure services on remote host |

### Benchmark Tasks

| Task | Description |
|------|-------------|
| `bench:start` | **[Agent]** Launch benchmark in background, return immediately with PID |
| `bench:status` | **[Agent]** Check progress. Exit `0` = done, exit `1` = running |
| `bench:stop` | **[Agent]** Stop metrics collection + collect logs |
| `bench:run` | [Sync] Run benchmark blocking until complete |
| `bench` | Alias for `bench:run` |
| `collect` | Pull results, metrics, and logs to local `results/` |

### Cleanup Tasks

| Task | Description |
|------|-------------|
| `clean` | Standard cleanup: stop → unmount → containers → images → data |
| `clean:full-rotate` | **Deep cleanup**: stop → clean all → rotate dirs → reboot |
| `clean:containers` | Remove benchmark containers |
| `clean:images` | Remove all images |
| `clean:mounts` | Unmount project-specific mounts + detach loop devices |
| `clean:snapshotter-data` | Remove metadata and snapshots |
| `clean:remote-data` | Remove remote temp files |
| `clean:rotate-dirs` | Rotate data dirs (timestamped) |
| `clean:reboot` | Reboot remote host + wait for recovery |

### Service Management

| Task | Description |
|------|-------------|
| `service:start` | Start service |
| `service:stop` | Stop service |
| `service:restart` | Restart all services |
| `service:status` | Show service status |

### Utility

| Task | Description |
|------|-------------|
| `validate` | Check `env.yml` exists and print config summary |
| `check` | Run environment checks |
| `ssh` | Open interactive SSH session |

---

## Agent Testing Workflows

### Workflow 1: First-Time Full Test

Use this when deploying from scratch or after code changes.

```
# Step 1: Build + Deploy + Configure
task build
task deploy
task check

# Step 2: Restart services
task service:restart

# Step 3: Launch benchmark (returns immediately)
task bench:start
# Output: "benchmark_started pid=<PID>"

# Step 4: Poll until complete (every 10-15 seconds)
task bench:status
# Exit code 1 → still running, wait and retry
# Exit code 0 → finished, proceed to step 5

# Step 5: Collect results
task bench:stop
task collect
```

**Or as a single command** (steps 1-3 combined):
```
task test:agent
# Then poll: task bench:status
# Then finalize: task bench:stop && task collect
```

### Workflow 2: Quick Re-Run (Skip Build/Deploy)

Use this to re-run benchmark with different parameters.

```
task clean:containers
task clean:images
task clean:mounts
task service:restart
task bench:start
task bench:status        # poll until exit 0
task bench:stop && task collect
```

**Or**: `task quick-bench:agent` then poll + collect.

### Workflow 3: Deep Clean + Benchmark (Pristine State)

Use this for **accurate benchmarking**. Ensures no leftover data.

```
task clean:full-rotate   # deep cleanup + reboot
task deploy:upload       # re-upload scripts
task deploy:configure    # re-configure
task service:restart
task bench:start
task bench:status        # poll until exit 0
task bench:stop && task collect
```

**Or**: `task clean-bench:agent` then poll + collect.

### Workflow 4: Change Parameters

```
# Edit env.yml (e.g., change benchmark.rate)
# Then use Workflow 2 or 3
task quick-bench:agent   # or task clean-bench:agent
task bench:status        # poll
task bench:stop && task collect
```

---

## `bench:status` Protocol

| Exit Code | Meaning | Agent Action |
|:---------:|---------|-------------|
| `0` | Benchmark finished | Proceed to `bench:stop` → `collect` |
| `1` | Still running | Wait 10-15 seconds, poll again |

### Structured Output

```
status=running|finished|no_benchmark
pid=<PID>
elapsed=<HH:MM:SS>
```

---

## Cleanup Strategy

### Standard (`clean`)
- Stop services → unmount → remove containers/images/metadata/temp files

### Deep (`clean:full-rotate`)
- Standard + rotate data dirs (timestamped) + reboot machine
- Guarantees: no page cache, no stale mounts, fresh databases

---

## Directory Layout

```
testing/
├── Taskfile.yml          # Main automation
├── env.example.yml       # Configuration template
├── env.yml               # Local config (git-ignored)
├── scripts/
│   ├── remote-setup.sh   # Remote: install, configure, check
│   ├── collect-metrics.sh # Remote: system metrics
│   └── collect-logs.sh   # Remote: log capture
└── results/              # Test results (git-ignored)
    └── run_YYYYMMDD_HHMMSS/
        ├── benchmark/    # JSON reports
        ├── metrics/      # System metrics
        └── logs/         # Service logs
```

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  Local Machine                    │
│                                                   │
│  task build      → compile binaries               │
│  task deploy     → SCP + SSH to remote            │
│  task bench:start → SSH (~2s, returns PID)        │
│  task bench:status → SSH (~1s, exit 0/1)          │
│  task bench:stop  → SSH (~5s)                     │
│  task collect     → SCP results to local          │
└──────────────────────────────────────────────────┘
          │ (optional: SSH -J bastion hop)
          ▼
┌──────────────────────────────────────────────────┐
│                  Remote Host                      │
│  {{REMOTE_LAYOUT}}                                │
└──────────────────────────────────────────────────┘
```

## Design Principles

- **File-based configuration** — all params in `env.yml`, no interactive prompts
- **Idempotent tasks** — safe to re-run any task
- **Clear exit codes** — non-zero on failure
- **Structured output** — JSON results, CSV metrics, key=value status
- **Async benchmark** — no long-running SSH; all calls return within seconds
- **Deep cleanup** — rotate data dirs + reboot for pristine state
