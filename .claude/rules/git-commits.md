# Git Commit Standards

All commits MUST follow the Conventional Commits specification.

## Format

```
<type>(<scope>): <short description>
```

## Allowed Types

| Type     | Meaning                                    |
|----------|--------------------------------------------|
| feat     | A new feature                              |
| fix      | A bug fix                                  |
| refactor | Code change (no bug fix, no new feature)   |
| perf     | Performance improvement                    |
| docs     | Documentation only changes                 |
| test     | Adding or updating tests                   |
| chore    | Maintenance (build scripts, tooling, deps) |
| ci       | CI/CD related changes                      |
| style    | Formatting changes (no logic changes)      |

## Scope Rules

- Describe the affected module: `server`, `storage`, `fuse`, `core`, `client`, `demo`, `rpc`, `test`
- Use lowercase, short identifiers
- Never leave scope empty unless the change is truly cross-cutting

## Description Rules

- Concise, <= 72 characters
- Imperative mood ("add", not "added")
- Start with lowercase
- No trailing period

## Co-Authored-By

Do NOT add `Co-Authored-By` lines to commit messages. Keep commit messages clean with only the type/scope/description.

## Commit Behavior

- **Atomic**: one logical change per commit
- **Small**: break large changes into incremental commits
- **Buildable**: every commit must compile (`cargo build --workspace`)
- **Tested**: never commit failing tests
- **Clean**: no debugging artifacts, no TODO comments left behind

## When to Commit

- After completing a single logical unit of work
- Before switching to a different task
- After fixing a bug (separate commit from feature work)
- After updating tests for new behavior
- During long tasks: commit at natural checkpoints (every 15-30 min of work)

## Proactive Commit Discipline

**IMPORTANT**: Agents MUST commit proactively. Do NOT wait for the user to ask.

- **After creating or modifying any file**: commit immediately once the change is logically complete. Do not accumulate uncommitted work across multiple files or steps.
- **During multi-step tasks**: commit after each completed step, not at the end. If a task has 5 steps, there should be up to 5 commits, not 1.
- **Before asking the user a question**: if there are uncommitted changes, commit them first. Never leave work uncommitted while waiting for user input.
- **Before running long operations** (builds, tests, benchmarks): commit all pending changes so progress is not lost.
- **After generating files** (scripts, configs, docs): commit immediately. Generated files are easy to forget.
- **Rule of thumb**: if you have done work that would be painful to redo, commit it now.

## Examples

```
feat(server): add streaming rpc support
fix(storage): handle empty metadata snapshot
refactor(fuse): extract mount helper utilities
test(server): add rpc integration tests
docs(guide): describe remote e2e testing workflow
chore(deps): bump fuser to 0.15
perf(storage): add bloom filter to directory CF
```
