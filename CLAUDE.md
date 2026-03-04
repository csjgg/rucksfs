# CLAUDE.md

RucksFS is a modular, trait-based FUSE filesystem in Rust, backed by RocksDB.
This is a graduation project. The demo binary is the final deliverable.

## Quick Reference

```bash
cargo build --workspace                    # Build all
cargo test --workspace                     # Run ~192 tests
cargo build --release -p rucksfs --features rocksdb  # Release binary
task --list                                # Show all task targets
task remote-test                           # Remote E2E pipeline
./scripts/e2e_fuse_test.sh                 # Local FUSE E2E test
```

## Architecture (5-crate workspace)

```
core       -> trait definitions (MetadataOps, DataOps, VfsOps)
storage    -> RocksDB + RawDisk backends
server     -> MetadataServer (namespace engine, delta compaction, PCC transactions)
dataserver -> DataServer (file I/O)
client     -> VfsCore router, EmbeddedClient, FuseClient
demo       -> standalone single-binary (rucksfs)
rpc        -> gRPC layer (tonic, unfinished)
```

## Key Design Decisions

- **Atomic mutations**: create/mkdir/unlink/rmdir/rename use AtomicWriteBatch with PCC, retry up to 3x on TransactionConflict
- **Delta-based updates**: parent dir timestamp/nlink changes are appended as DeltaOp, folded on read, compacted in background when > 32 deltas
- **FUSE permissions**: default_permissions + AllowOther mount options let kernel VFS handle POSIX permission checks
- **Root inode**: inode 1 (ROOT_INODE), auto-initialized on MetadataServer construction
- **Binary encoding**: big-endian keys for byte-order == numeric-order (see storage/src/encoding.rs)
- **Platform**: FUSE support is cfg(target_os = linux) only

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new metadata operation | core/src/lib.rs (traits) -> server/src/lib.rs (impl) -> client/src/embedded.rs + vfs_core.rs (passthrough) -> client/src/fuse.rs (FUSE bridge) |
| Modify storage format | storage/src/encoding.rs (key format) + storage/src/rocks.rs (RocksDB backend) |
| Change mount behavior | client/src/fuse.rs (mount options) + demo/src/main.rs (CLI args) |
| Add tests | demo/tests/integration_test.rs (full-stack) or server/tests/ (metadata-only) |
| Remote E2E testing | Taskfile.yml (workflow) + scripts/remote-test/ (scripts) |

## Documentation

- docs/design.md: Full system design (120KB, consult sections as needed, do NOT read entirely)
- docs/guide.md: Deployment and usage guide
- docs/TODO.md: Structured task list with priorities
- docs/standalone-analysis.md: Architecture comparison with JuiceFS
- docs/metadata-kv-research.md: Research paper survey on metadata KV storage

## Agent Behavior Rules

**IMPORTANT**: Follow these rules for all interactions with this codebase.

### 1. Think Before Acting

- **Do NOT rush into long execution tasks.** When information is incomplete, ask the user short clarifying questions first.
- Before starting a multi-step implementation, present a brief plan and confirm with the user.
- If stuck on configuration issues, remote access problems, or ambiguous requirements, ask the user rather than guessing.

### 2. Information Gathering Strategy

- Read docs/TODO.md to understand current priorities before starting work.
- Consult docs/design.md section-by-section (it is 120KB, never read the whole thing at once).
- Check docs/guide.md for deployment procedures before modifying FUSE/mount/CLI behavior.
- When modifying a trait, trace all implementations: core -> server -> client/embedded -> client/vfs_core -> client/fuse -> demo.

### 3. Code Modification Rules

- **Minimal changes**: only modify files necessary for the task.
- **Preserve style**: match existing indentation, naming, and patterns.
- **No breaking changes** unless explicitly requested.
- Comments in English; conversation in Chinese.
- After non-trivial changes, run cargo test --workspace to verify.

### 4. Git Commits

Follow Conventional Commits. See .claude/rules/git-commits.md for full specification.

Quick reference:
```
feat(server): add streaming rpc support
fix(storage): handle empty metadata snapshot
refactor(fuse): simplify mount logic
test(demo): add concurrent rename test
```

Commit after each logical unit of work. Each commit must compile and pass tests.

### 5. Testing Workflow

```
1. Unit tests     -> cargo test --workspace
2. Local E2E      -> ./scripts/e2e_fuse_test.sh
3. Remote E2E     -> task remote-test
4. Bench only     -> task remote-test:bench-only
5. Quick verify   -> task remote-test:quick
```

### 6. Known Pitfalls

- rpc crate requires protoc v3.15+. Skip with cargo test --workspace (it is excluded from default members)
- RocksDB builds slowly; use cargo test -p rucksfs-server for fast iteration
- FUSE tests need Linux + /dev/fuse + user_allow_other in /etc/fuse.conf
- design.md is 120KB. Use section search, never load entirely
