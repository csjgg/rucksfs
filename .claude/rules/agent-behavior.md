# Agent Behavior Rules

These rules govern how an AI agent should interact with the RucksFS codebase.

## Communication Strategy

### Short Dialogue First

Before starting any multi-step task (>3 steps), the agent MUST:

1. **Summarize** the proposed approach in 3-5 bullet points
2. **Ask** the user to confirm or adjust
3. **Only then** begin execution

This is ESPECIALLY important for:
- Architecture changes (new traits, new crates)
- Refactors that touch >3 files
- Anything involving the storage layer (risk of data loss)
- Remote deployment or SSH operations

### When to Ask vs. When to Act

**Ask the user when:**
- Requirements are ambiguous or have multiple valid interpretations
- Configuration values are needed (SSH credentials, paths, ports)
- A proposed change would break existing tests
- Stuck after 2 failed attempts at the same approach
- Multiple equally valid approaches exist — let the user choose

**Act independently when:**
- The task is clearly defined with a single obvious approach
- Fixing a compilation error caused by your own edit
- Running tests to verify changes
- Reading documentation or source code for context

### Error Recovery

- If a command fails, diagnose before retrying (don't blindly retry 3 times)
- If an edit tool fails, read the current file content first, then retry
- If compilation fails after your change, fix it immediately
- After 3 failed attempts at the same thing, stop and ask the user

## Information Gathering Protocol

Before starting work on any task:

1. **Check TODO.md** — understand current priorities and what's already done
2. **Read related source files** — understand the current implementation
3. **Consult design.md by section** — never read the entire 120KB file
4. **Check guide.md** — for deployment and configuration context
5. **Trace trait chains** — when modifying interfaces, follow: `core` -> `server` -> `client/embedded` -> `client/vfs_core` -> `client/fuse` -> `demo`

## Task Execution Guidelines

### For Bug Fixes
1. Reproduce the issue (or understand the report)
2. Identify root cause in source code
3. Fix with minimal changes
4. Add/update test if applicable
5. Verify with `cargo test`
6. Commit

### For New Features
1. Discuss approach with user (short dialogue)
2. Check design.md for relevant section
3. Implement in dependency order: core -> storage -> server -> client -> demo
4. Add tests
5. Update docs/TODO.md
6. Commit incrementally

### For Refactors
1. Explain the motivation and scope
2. Confirm with user
3. Make changes in small, atomic commits
4. Ensure tests pass at each step

## Long Task Checkpoints

During multi-step tasks, commit and report progress at these checkpoints:
- After modifying trait definitions
- After implementing server-side logic
- After connecting client layer
- After all tests pass
- After updating documentation
