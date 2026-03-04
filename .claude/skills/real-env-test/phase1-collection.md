# Phase 1: Information Collection

Detailed instructions for the information-gathering phase of the
`real-env-test` skill.

## Overview

Before generating any files, you must understand:
1. How the project is built
2. How it is deployed and configured
3. What services it runs
4. What to test and what metrics to collect
5. SSH access details for the remote host

## Mode Selection: Fast vs Full

Before asking questions, scan the project automatically.

### Auto-Scan Checklist

| Check | What to look for |
|-------|------------------|
| Build system | `Makefile`, `Cargo.toml`, `go.mod`, `CMakeLists.txt`, `package.json` (with build script) |
| Deploy docs | README deploy/install section, `docs/deploy.md`, `INSTALL.md` |
| Systemd units | `*.service` files in `packaging/`, `deploy/`, `systemd/`, or project root |
| Config templates | `*.toml.example`, `*.yml.example`, `config.toml.sample`, etc. |
| Existing tests | `testing/`, `tests/`, `benchmarks/`, `bench/` directories |
| CLI help | Binary `--help` output, `configure-containerd` subcommands, etc. |

### Decision Matrix

| Project docs complete? | User intent clear? | Mode |
|:----------------------:|:------------------:|:----:|
| ✅ | ✅ | **Fast** |
| ✅ | ❌ | **Semi** — ask only about test specifics |
| ❌ | ✅ | **Semi** — ask only about deployment gaps |
| ❌ | ❌ | **Full** |

**Criteria for "complete docs"**: Has build instructions + install steps +
config file examples + service management commands.

**Criteria for "clear user intent"**: User specified what to test (e.g.,
"benchmark image pull performance"), the target environment, and basic
parameters.

---

## Fast Mode (≤3 Questions)

When both project docs and user intent are clear, only confirm:

1. **SSH credentials**: host, user, key path, bastion (if needed)
2. **Test type + key parameters**: benchmark mode, rate, duration, dataset
3. **Cleanup preference**: standard vs deep (with directory rotation + reboot)

For everything else, **infer from project docs and source code**, then present
a summary for confirmation:

```
Based on my analysis of the project, here's what I'll configure:

Build:
  - Command: `cross build --release --target x86_64-unknown-linux-musl`
  - Binaries: erofs-snapshotter, ctr-benchmark

Deploy:
  - Install to: /usr/local/bin/
  - Config: /etc/erofs-snapshotter/config.toml
  - Service: erofs-snapshotter.service (systemd)
  - Containerd config: via `erofs-snapshotter configure-containerd`

Benchmark:
  - Mode: erofs
  - Rate: 1 container/s
  - Duration: 60s

Does this look correct? Any changes needed?
```

---

## Semi Mode

Ask only the categories where information is missing. For categories with
sufficient documentation, show inferred values inline.

Example (docs complete, intent unclear):
```
I've analyzed the project's build and deploy process. I need a few more
details about the test you want to run:

1. What type of test? (e.g., performance benchmark, stress test, correctness)
2. Key parameters? (rate, duration, concurrency)
3. What metrics matter most? (latency, throughput, resource usage)
```

Example (intent clear, docs incomplete):
```
You want a performance benchmark at 20 containers/s. I need some deployment
details I couldn't find in the docs:

1. How do you build the binary? (I see a Cargo.toml but no cross-compile config)
2. Where should the config file be installed? (I found a template but no path)
3. How is the service managed? (systemd? supervisor? manual?)
```

---

## Full Mode (7-Category Questionnaire)

When both docs and intent are unclear, work through these categories. Ask in
**grouped batches** (2-3 categories at a time), not all at once.

### Batch 1: Environment + Prerequisites

**Category 1: Target Environment**
- Remote host IP/hostname
- SSH user, key path, port
- Bastion/jump host (if any)
- OS and architecture (e.g., Linux x86_64, aarch64)

**Category 2: Prerequisites**
- Required dependencies on remote host
- Pre-mounted storage (e.g., CFS, NFS)
- Kernel version requirements
- Network access requirements (registries, etc.)

### Batch 2: Build + Deploy

**Category 3: Build Process**
- Build command(s) and toolchain
- Cross-compilation requirements
- Build targets/output paths
- Multiple binaries? (e.g., main binary + benchmark tool)

**Category 4: Deployment**
- Binary install paths
- Config file locations and format
- Service management (systemd unit, manual start)
- Config generation commands (e.g., `binary configure-xxx`)
- Dependencies to configure (e.g., containerd, docker)

### Batch 3: Test + Metrics + Cleanup

**Category 5: Test Specification**
- Test type (performance, stress, correctness, smoke)
- Key parameters (rate, duration, concurrency, dataset)
- Test binary and its arguments
- Expected runtime

**Category 6: Metrics Collection**
- What to measure (CPU, memory, disk I/O, network)
- Application-specific metrics (Prometheus endpoint?)
- Latency breakdowns (p50/p95/p99)
- Log sources to capture

**Category 7: Cleanup Strategy**
- Pre-test cleanup requirements
- What artifacts to remove between runs
- Whether data directory rotation is needed
- Whether machine reboot is needed for clean state

---

## Project Reading Strategy

When scanning the project, read in this order:

1. **Root-level files**: README.md, Makefile, Cargo.toml, go.mod
2. **Build configuration**: Cross.toml, .cargo/config.toml, Dockerfile
3. **Packaging/deployment**: `packaging/`, `deploy/`, systemd units
4. **Config templates**: Any `*.example`, `*.sample`, `*.template` files
5. **CLI help**: If a binary exists, check for `--help` or subcommand docs
6. **Existing tests**: `testing/`, `tests/`, `benchmarks/` directories

### What to Extract

| From | Extract |
|------|---------|
| Build files | Build commands, targets, output paths |
| README | Install steps, config instructions, quick start |
| Systemd units | Service name, ExecStart path, dependencies |
| Config templates | All configurable fields, default values, file format |
| CLI help | Subcommands (e.g., `configure-containerd`), flags |
| Existing tests | Current test approach, what's missing |

---

## Output of Phase 1

At the end of Phase 1, you should have a complete **configuration profile**:

```yaml
# Internal reference — not written to a file
project:
  name: "<project name>"
  language: "<rust|go|python|...>"
  build_command: "<command>"
  build_targets:
    - name: "<binary name>"
      path: "<output path>"
      cross_compile: <true|false>
      target: "<e.g., x86_64-unknown-linux-musl>"

remote:
  host: "<ip>"
  user: "<user>"
  ssh_key: "<path>"
  port: <22>
  bastion: <null or {host, user, ssh_key, port}>

deploy:
  install_path: "<e.g., /usr/local/bin/>"
  config_path: "<e.g., /etc/project/config.toml>"
  config_command: "<e.g., binary configure-containerd>"
  service_name: "<e.g., erofs-snapshotter>"
  service_file: "<path to .service file>"
  dependencies: ["<e.g., containerd>"]

test:
  type: "<benchmark|stress|smoke>"
  binary: "<test binary name>"
  parameters:
    mode: "<mode>"
    rate: <N>
    duration: <seconds>
    dataset: "<path>"
    namespace: "<namespace>"

metrics:
  system: [cpu, memory, disk_io]
  application: "<prometheus endpoint or null>"
  logs: ["<service1>", "<service2>"]

cleanup:
  strategy: "<standard|deep>"
  rotate_dirs: <true|false>
  reboot: <true|false>
  pre_mount: "<e.g., CFS mount point>"
```

Present this to the user for final approval before proceeding to Phase 2.
