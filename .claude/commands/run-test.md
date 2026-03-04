---
name: "Run Test"
description: "Execute a testing workflow using the test-runner sub-agent"
---

**Steps**

1. Locate the `testing/` directory in the current project. If not found, tell the user to run `/real-env-test` first to generate the testing framework.

2. Verify prerequisites exist:
   - `testing/Taskfile.yml`
   - `testing/env.yml` (not `env.example.yml` — the actual config)
   - `testing/README.md`

3. Read `testing/README.md` to understand available workflows.

4. Determine which workflow to run:
   - If the user specified a workflow (e.g., "workflow 3", "deep clean bench"), use that.
   - If the user specified parameters (e.g., "rate=20"), note them as overrides.
   - If the user said "run the test" without specifics, check context:
     - First time or code changed → suggest Workflow 1 (full test)
     - Re-running same binary → suggest Workflow 2 (quick re-run)
     - Need accurate results → suggest Workflow 3 (deep clean)

5. Spawn the `test-runner` sub-agent with a message like:

   ```
   Execute testing workflow for this project.

   Working directory: <absolute path>/testing/
   Workflow: <number>
   Parameter overrides: <if any>

   Read the README.md in the testing directory for complete workflow instructions.
   Follow the async benchmark protocol: bench:start → poll bench:status → bench:stop → collect.
   ```

6. After `test-runner` completes:
   - If success: Read the results and provide analysis to the user (key metrics, comparison with previous runs if available, recommendations).
   - If error: Read the error report, diagnose the issue, and either fix the framework or ask the user for guidance.

**Guardrails**
- Do NOT run test commands directly — always delegate to the `test-runner` sub-agent.
- Do NOT modify Taskfile.yml or scripts unless `test-runner` reports a framework bug.
- If `env.yml` is missing, help the user create it from `env.example.yml` before proceeding.
- Parameter overrides (rate, duration, mode) can be passed to `test-runner` for it to apply.
