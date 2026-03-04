---
name: real-env-test
description: Generate and execute real-environment deployment tests via Taskfile + SSH
disable-model-invocation: true
---

# Real-Environment Test Skill

Generate Taskfile-based testing frameworks for projects that need **real remote
environment deployment testing** — build → deploy → benchmark → collect results,
all automated via SSH.

## When This Skill Applies

Use `/real-env-test` when:
- A project needs performance or integration testing on a **real remote host**
- Testing involves deploying binaries, configuring services, running benchmarks
- Results must be collected as structured data (JSON, CSV)

Do NOT use when:
- Unit tests or local integration tests suffice
- No remote host is involved
- The project already has a complete testing framework (use `test-runner` agent instead)

## Mechanism Clarification

| Component | Type | Purpose |
|-----------|------|---------|
| This file (SKILL.md) | **Skill** — injected instructions | Guides the current session through framework generation |
| `test-runner` | **Sub-agent** — independent context | Executes tests following the generated README workflows |
| `/run-test` | **Command** — user-triggered | Directly invokes test-runner for existing frameworks |

**Key distinction**: This Skill runs inside the current session. It is NOT a
separate process. The sub-agent `test-runner` IS a separate context window.

## Four-Phase Workflow

```
Phase 1: Information Collection (this session)
    ↓
Phase 2: Framework Generation (this session)
    ↓
Phase 3: Test Execution (delegated to test-runner sub-agent)
    ↓
Phase 4: Result Analysis (back in this session)
```

---

## Phase 1: Information Collection

**Goal**: Understand the project and gather everything needed to generate the
testing framework.

**Detailed instructions**: Read `phase1-collection.md` in this directory.

### Summary

1. **Auto-scan the project**: Read build files (Makefile, Cargo.toml, go.mod),
   deployment docs (README, INSTALL.md), systemd units, config templates.

2. **Evaluate completeness** and select mode:

   | Project docs | User intent | Mode |
   |:------------:|:-----------:|:----:|
   | Complete | Clear | **Fast** — ≤3 questions |
   | Partial  | Clear | **Semi** — ask only about gaps |
   | Complete | Vague | **Semi** — ask only about test specifics |
   | Missing  | Vague | **Full** — 7-category questionnaire |

3. **Collect missing information** via multi-turn dialogue.

4. **Present a summary** of all inferred + confirmed values for user approval.

---

## Phase 2: Framework Generation

**Goal**: Generate a complete `testing/` directory with all files needed for
automated testing.

**Detailed instructions**: Read `phase2-generation.md` in this directory.

### Summary

Generate the following directory structure:

```
<project>/testing/
├── Taskfile.yml          # All task definitions
├── env.example.yml       # Configuration template (committed)
├── env.yml               # User's local config (git-ignored)
├── .gitignore            # Ignore env.yml and results/
├── scripts/
│   ├── remote-setup.sh   # Install + configure on remote
│   ├── collect-metrics.sh # System metrics sampling
│   └── collect-logs.sh   # Service log capture
├── results/              # Test results (git-ignored)
└── README.md             # Agent-friendly workflow documentation
```

**Templates** are in the `templates/` subdirectory of this skill. Use them as
the skeleton and fill in project-specific values.

### Generation Rules

1. Use `templates/Taskfile.yml.tpl` — replace `{{PLACEHOLDER}}` markers
2. Use `templates/README.md.tpl` — fill in project-specific sections
3. Use `templates/env.example.yml` — adjust fields for the project
4. Generate `scripts/` based on project's deploy/install requirements
5. Create `.gitignore` with `env.yml` and `results/`

---

## Phase 3: Test Execution

**Goal**: Execute the generated testing workflows on the remote host.

**This phase is delegated to the `test-runner` sub-agent** (defined in
`.claude/agents/test-runner.md`).

### When to Delegate

- After Phase 2 is complete AND user has filled in `env.yml`
- User explicitly requests execution (e.g., "run the test now")
- User uses `/run-test` command directly

### How to Delegate

Spawn the `test-runner` sub-agent with the following message:

```
Execute testing workflow for project at <path>.

Working directory: <project>/testing/
Workflow: <1|2|3|4> (or specific task commands)
Parameter overrides: <if any, e.g., rate=20, duration=120>

Read the README.md in the testing directory for complete workflow instructions.
```

### What test-runner Does

1. Reads `testing/README.md` for available workflows
2. Reads `testing/env.yml` for current configuration
3. Runs `task validate` to confirm setup
4. Executes the specified workflow step by step
5. Uses async protocol: `bench:start` → poll `bench:status` → `bench:stop`
6. Collects results via `task collect`
7. Returns a structured summary

### What test-runner CANNOT Do

- Spawn sub-agents (it is a leaf executor)
- Modify Taskfile.yml or scripts
- Ask the user questions
- Make autonomous decisions about failures (must escalate)

---

## Phase 4: Result Analysis

**Goal**: Interpret the test results and report findings to the user.

After `test-runner` completes:

1. **Read result files** in `testing/results/run_<timestamp>/`:
   - `benchmark/*.json` — benchmark reports
   - `metrics/` — system resource usage (CPU, memory, disk I/O)
   - `logs/` — service logs

2. **Extract key metrics**:
   - Total containers created / failed
   - Average pull time, create time, start time
   - p50 / p95 / p99 latencies
   - Throughput (containers/second actual vs target)
   - Error rate and error categories
   - Peak CPU / memory / disk I/O

3. **Identify issues**:
   - Performance bottlenecks
   - Error patterns
   - Resource saturation points

4. **Report to user** with:
   - Summary table of key metrics
   - Comparison with previous runs (if available)
   - Specific recommendations

---

## Agent-Friendly Design Requirements

All generated testing frameworks MUST follow these principles:

### SSH Constraints
- Every SSH command must complete within 30 seconds
- No interactive prompts — all params come from `env.yml`
- Use async pattern for long-running operations (nohup + PID + polling)

### Async Benchmark Protocol
```
bench:start  → launches in background, returns PID within ~2s
bench:status → exit 0 = done, exit 1 = running (returns within ~1s)
bench:stop   → finalizes collection (returns within ~5s)
collect      → pulls results to local (returns within ~10s)
```

### Output Format
- Structured key=value pairs for status (e.g., `status=running pid=12345`)
- JSON for benchmark results
- CSV for metrics
- Clear exit codes: 0 = success, non-zero = failure

### Idempotency
- Every task is safe to re-run
- `clean` tasks remove all artifacts from previous runs
- `deploy` tasks overwrite previous installations

### Cleanup Strategy
Two levels:
1. **Standard** (`clean`): Stop services, remove containers/images/metadata
2. **Deep** (`clean:full-rotate`): Standard + rotate data dirs + reboot machine

---

## Session vs Sub-agent Responsibility

| Action | Who |
|--------|-----|
| Read project source code and docs | **Session** (Phase 1) |
| Multi-turn dialogue with user | **Session** (Phase 1) |
| Generate testing/ framework files | **Session** (Phase 2) |
| Fix framework bugs reported by test-runner | **Session** |
| Execute `task` commands via SSH | **test-runner** (Phase 3) |
| Poll benchmark status | **test-runner** (Phase 3) |
| Collect results from remote | **test-runner** (Phase 3) |
| Analyze results and report | **Session** (Phase 4) |
| Decide retry/fix/abort on errors | **Session** |

---

## Error Handling

When `test-runner` reports an error:

1. Read the structured error report:
   ```
   error_phase=<deploy|benchmark|collect>
   error_task=<task name>
   error_exit_code=<code>
   error_message=<stderr content>
   attempted_cleanup=<true|false>
   ```

2. Diagnose the root cause (read logs if needed)

3. Decide action:
   - **Fix framework**: Edit Taskfile.yml or scripts, re-deploy
   - **Retry**: Re-spawn test-runner with same parameters
   - **Abort**: Report failure to user with diagnosis

4. Never let test-runner retry autonomously — it must escalate.

---

## Reference Implementation

The `testing/` directory in this project (erofs-snapshotter) is a complete
reference implementation of this skill's output. Examine it for conventions:
- `testing/Taskfile.yml` — full task definitions
- `testing/README.md` — agent-friendly workflow documentation
- `testing/env.example.yml` — configuration template
- `testing/scripts/` — remote execution scripts
