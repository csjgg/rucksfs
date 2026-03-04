# Phase 2: Framework Generation

Detailed instructions for generating the testing framework directory.

## Overview

Using the configuration profile from Phase 1, generate a complete `testing/`
directory. Every generated file must be immediately usable — no manual patching.

## Directory Structure

```
<project>/testing/
├── Taskfile.yml          # All task definitions (main automation)
├── env.example.yml       # Configuration template (committed to git)
├── env.yml               # User's actual config (git-ignored)
├── .gitignore            # Ignore env.yml and results/
├── scripts/
│   ├── remote-setup.sh   # Remote: install, configure, check
│   ├── collect-metrics.sh # Remote: system load sampling
│   └── collect-logs.sh   # Remote: service log capture
├── results/              # Test results (git-ignored)
└── README.md             # Agent-friendly workflow docs
```

## File-by-File Generation Rules

### 1. Taskfile.yml

**Template**: Use `templates/Taskfile.yml.tpl` as skeleton.

**Replace placeholders**:

| Placeholder | Source (Phase 1 profile) |
|-------------|-------------------------|
| `{{PROJECT_NAME}}` | `project.name` |
| `{{BUILD_COMMANDS}}` | `project.build_command` + targets |
| `{{BINARY_NAMES}}` | `project.build_targets[*].name` |
| `{{DEPLOY_INSTALL_CMD}}` | Install commands from deploy config |
| `{{DEPLOY_CONFIGURE_CMD}}` | `deploy.config_command` |
| `{{SERVICE_NAME}}` | `deploy.service_name` |
| `{{BENCH_BINARY}}` | `test.binary` |
| `{{BENCH_ARGS}}` | `test.parameters` mapped to CLI flags |
| `{{DEPENDENCIES}}` | `deploy.dependencies` |
| `{{METRICS_ENDPOINT}}` | `metrics.application` |

**Required task categories** (every Taskfile must have all of these):

| Category | Tasks |
|----------|-------|
| Validation | `validate` |
| Build | `build`, `build:<component>` for each binary |
| Deploy | `deploy`, `deploy:upload`, `deploy:install`, `deploy:configure` |
| Check | `check` |
| Service | `service:start`, `service:stop`, `service:restart`, `service:status` |
| Benchmark (sync) | `bench:run`, `bench` (alias) |
| Benchmark (async) | `bench:start`, `bench:status`, `bench:stop` |
| Collect | `collect` |
| Cleanup | `clean`, `clean:full-rotate`, `clean:containers`, `clean:images`, `clean:mounts`, `clean:rotate-dirs`, `clean:reboot` |
| Composite (sync) | `test`, `quick-bench` |
| Composite (async) | `test:agent`, `quick-bench:agent`, `clean-bench:agent` |
| Utility | `ssh` |

**SSH/SCP helper pattern**:
```yaml
vars:
  SSH_OPTS: "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
  # ... load from env.yml via yq with grep/awk fallback

env:
  SSH_CMD: 'ssh {{.SSH_OPTS}} {{.PROXY_JUMP_FLAG}} -i {{.REMOTE_SSH_KEY}} -p {{.REMOTE_PORT}} {{.REMOTE_USER}}@{{.REMOTE_HOST}}'
  SCP_CMD: 'scp {{.SSH_OPTS}} {{.PROXY_JUMP_FLAG}} -i {{.REMOTE_SSH_KEY}} -P {{.REMOTE_PORT}}'
```

**Bastion support**: Always include bastion/jump host variables and `PROXY_JUMP_FLAG`
derivation, even if bastion is disabled by default.

### 2. env.example.yml

**Template**: Use `templates/env.example.yml` as skeleton.

**Rules**:
- Every field must have a comment explaining its purpose
- Default values should be sensible for the project
- Sensitive fields (host, SSH key) should have placeholder values
- Group fields by section: `remote`, `bastion`, `cfs` (or project-specific storage),
  `<service>` (main service config), `benchmark`, `build`

### 3. README.md

**Template**: Use `templates/README.md.tpl` as skeleton.

**Required sections** (in this order):

1. **Title + one-line description**
2. **Agent-First Design callout** (blockquote)
3. **Prerequisites** (table: tool + purpose)
4. **Quick Start** (3-line code block: cp env, edit, task test)
5. **Configuration** (table of env.yml sections)
6. **Available Tasks** (grouped tables: composite, build, deploy, bench, clean, service, utility)
7. **Agent Testing Workflows** (4 workflows with step-by-step commands)
   - Workflow 1: First-Time Full Test
   - Workflow 2: Quick Re-Run
   - Workflow 3: Deep Clean + Benchmark
   - Workflow 4: Parameter Change
8. **bench:status Protocol** (exit codes + structured output format)
9. **Cleanup Strategy** (standard vs deep)
10. **Directory Layout** (tree diagram)
11. **Architecture** (ASCII diagram: local → bastion → remote)
12. **Design Principles** (bullet list)

### 4. scripts/remote-setup.sh

**Structure**: Single script with subcommands.

```bash
#!/bin/bash
set -euo pipefail

ACTION="${1:-help}"

case "${ACTION}" in
  install)     do_install ;;
  configure-*) do_configure_xxx ;;
  check)       do_check ;;
  *)           echo "Usage: $0 {install|configure-xxx|check}" ;;
esac
```

**install**: Copy binaries to install paths, install systemd unit, daemon-reload.
**configure-xxx**: Generate/update config files. Use the project's own config
commands where available (e.g., `erofs-snapshotter configure-containerd`).
**check**: Verify all prerequisites (services running, mounts present, binaries
exist, config valid).

### 5. scripts/collect-metrics.sh

**Structure**: Start/stop pattern for background metrics collection.

```bash
#!/bin/bash
# Usage: collect-metrics.sh start <output_dir> <interval_seconds>
#        collect-metrics.sh stop
```

**Metrics to collect**:
- CPU usage (from `/proc/stat` or `mpstat`)
- Memory usage (from `/proc/meminfo`)
- Disk I/O (from `iostat` or `/proc/diskstats`)
- Application metrics (from Prometheus endpoint if configured)

### 6. scripts/collect-logs.sh

**Structure**: Capture logs from relevant services.

```bash
#!/bin/bash
# Usage: collect-logs.sh <output_dir> <duration_seconds>
```

**Logs to collect** (project-specific):
- Main service journal logs (`journalctl -u <service>`)
- Dependency service logs (`journalctl -u containerd`)
- Kernel messages (`dmesg`)
- Application-specific log files

### 7. .gitignore

```
env.yml
results/
```

---

## Code Quality Rules

### Shell Scripts
- Always start with `#!/bin/bash` and `set -euo pipefail`
- Use functions for each action
- Echo progress markers: `echo "==> Doing something..."`
- Handle errors explicitly, don't silently fail
- Quote all variables: `"${VAR}"`

### Taskfile.yml
- Use `silent: true` for validation tasks
- Use `deps: [validate]` for tasks that need config
- Every task must have a `desc:` field
- Use `$SSH_CMD` and `$SCP_CMD` from env section
- Long SSH commands should be multi-line with proper escaping

### README.md
- Every code block must be copy-pasteable
- No placeholders in example commands
- Use tables for structured information
- Exit code documentation is mandatory for agent-facing tasks

---

## Async Benchmark Pattern

This is the most critical pattern. Every generated Taskfile MUST implement:

### bench:start
```yaml
bench:start:
  desc: "[Agent] Start benchmark in background (returns immediately with PID)"
  cmds:
    - # Start metrics collection in background
    - # Launch benchmark via nohup, save PID to file
    - # Verify process started (check PID exists)
    - # Print: benchmark_started pid=<PID>
```

### bench:status
```yaml
bench:status:
  desc: "[Agent] Check benchmark status (exit 0 = finished, exit 1 = running)"
  cmds:
    - # Check if PID file exists
    - # If PID running: print status=running + exit 1
    - # If PID finished: print status=finished + last N lines + exit 0
```

### bench:stop
```yaml
bench:stop:
  desc: "[Agent] Stop metrics collection and collect logs"
  cmds:
    - # Stop metrics collector
    - # Collect logs
    - # Print: post_benchmark_collection_complete
```

**Critical**: `bench:status` exit code is the contract:
- Exit 0 = benchmark finished (agent proceeds to collect)
- Exit 1 = still running (agent waits and polls again)

---

## Cleanup Pattern

### Standard (clean)
```
service:stop → clean:mounts → clean:containers → clean:images →
clean:snapshotter-data → clean:remote-data
```

### Deep (clean:full-rotate)
```
Standard cleanup → clean:rotate-dirs → clean:reboot
```

**rotate-dirs** must:
1. Create timestamped directories for all data paths
2. Update config files to point to new directories
3. Leave old data intact (just renamed)

**reboot** must:
1. Send `reboot` command
2. Wait in a loop (with max retries) for host to come back
3. Verify host is up with a test SSH command
4. Wait for services to stabilize

---

## Validation Checklist

After generating all files, verify:

- [ ] `task validate` works with a valid env.yml
- [ ] All task names match the required categories above
- [ ] `bench:status` returns exit 0 on completion, exit 1 while running
- [ ] `bench:start` returns within ~2 seconds
- [ ] SSH commands use `$SSH_CMD` / `$SCP_CMD` pattern
- [ ] Bastion support is included (even if disabled by default)
- [ ] README workflows match actual task names
- [ ] `.gitignore` includes `env.yml` and `results/`
- [ ] Scripts have `set -euo pipefail`
- [ ] All env.yml fields have comments in env.example.yml
