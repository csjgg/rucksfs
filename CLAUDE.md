# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RucksFS is a modular, trait-based user-space file system in Rust, inspired by JuiceFS. It separates metadata and data paths: MetadataServer manages the namespace, DataServer stores file contents, and clients route operations through a VFS layer.

## Build & Test Commands

```bash
# Build entire workspace
cargo build --workspace

# Run all tests (~192 tests, excludes rpc due to protoc version requirement)
cargo test --workspace

# Run tests for a specific crate
cargo test -p rucksfs-server
cargo test -p rucksfs
cargo test -p rucksfs-dataserver
cargo test -p rucksfs-storage

# Run a single test by name
cargo test -p rucksfs-server -- test_name

# Run the standalone filesystem (auto-demo, default data dir: ~/.rucksfs)
cargo run -p rucksfs

# Run with a custom data directory
cargo run -p rucksfs -- --data-dir /tmp/rucksfs-data

# Interactive REPL mode
cargo run -p rucksfs -- --interactive

# FUSE mount (Linux only)
cargo run -p rucksfs -- --mount /mnt/rucksfs

# E2E FUSE tests (Linux, requires FUSE support)
./scripts/e2e_fuse_test.sh
```

## Architecture

### Crate Dependency Graph

```
core  (trait definitions + shared types, no dependencies)
  ↑
storage  (MetadataStore, DataStore, DirectoryIndex, DeltaStore traits + RocksDB/RawDisk impls)
  ↑
server  (MetadataServer — namespace engine)
dataserver  (DataServer<D: DataStore> — data I/O engine)
  ↑
client  (VfsCore router, EmbeddedClient, FuseClient)
rpc  (gRPC layer with tonic, protobuf in rpc/proto/)
  ↑
demo  (standalone single-binary `rucksfs`, RocksDB-only)
```

### Key Traits (defined in `core/src/lib.rs`)

- **`MetadataOps`** — Namespace operations (lookup, create, mkdir, unlink, rename, setattr, open, report_write). Implemented by `MetadataServer`.
- **`DataOps`** — File I/O (read_data, write_data, truncate, flush, delete_data). Implemented by `DataServer`.
- **`VfsOps`** — Full POSIX VFS interface combining metadata + data ops. Implemented by `EmbeddedClient` (in-process) and future `RucksClient` (network).

### Storage Traits (defined in `storage/src/lib.rs`)

- **`MetadataStore`** — KV store for inode metadata (get/put/delete/scan_prefix)
- **`DirectoryIndex`** — Directory entry resolution (resolve_path, list_dir, insert_child, remove_child)
- **`DeltaStore`** — Append-only delta store for incremental inode attribute updates
- **`DataStore`** — Async file data I/O (read_at, write_at, truncate, flush, delete)
- **`StorageBundle`** — Creates `AtomicWriteBatch` for cross-store atomic writes
- **`AtomicWriteBatch`** — Collects `BatchOp` variants and commits atomically; supports `get_for_update_*` for pessimistic concurrency control (PCC)

Backends: `Rocks*` (metadata/directory/delta stores) + `RawDiskDataStore` (file data). RocksDB is an unconditional dependency of the `storage` crate — no feature flag needed.

### Data Flow

1. **Write path**: Client → `DataServer::write_data` → then `MetadataServer::report_write` (updates size/mtime)
2. **Read path**: Client → `DataServer::read_data` (bypasses MetadataServer)
3. **Metadata mutations** (create/mkdir/unlink/rmdir/rename): Use `AtomicWriteBatch` with PCC transactions, retry up to 3 times on `TransactionConflict`

### Delta-Based Metadata Updates

Instead of read-modify-write on the base inode for every operation, parent directory timestamp/nlink changes are appended as `DeltaOp` entries (defined in `server/src/delta.rs`). On read, deltas are folded into the base value. Background `DeltaCompactionWorker` (`server/src/compaction.rs`) periodically merges deltas when they exceed a threshold (default 32).

### Binary Encoding (storage/src/encoding.rs)

All KV keys use big-endian encoding for byte-order == numeric-order:
- Inode key: `[b'I'][inode: u64 BE]` (9 bytes)
- Dir entry key: `[b'D'][parent: u64 BE][name: UTF-8]`
- Delta key: `[b'X'][inode: u64 BE][seq: u64 BE]` (17 bytes)
- InodeValue: 57-byte version-tagged binary blob

### Wiring It Together

See `demo/src/main.rs` for how components are assembled. Pattern:
1. Create RocksDB + RawDisk storage backends
2. Wrap data store in `DataServer` → `Arc<dyn DataOps>`
3. Create `RocksStorageBundle` for atomic writes
4. Create `MetadataServer` with all stores + data client + bundle
5. Create `EmbeddedClient` with metadata + data references

### Storage Backends

The `storage` crate provides a single set of production backends:
- `RocksMetadataStore`, `RocksDirectoryIndex`, `RocksDeltaStore`, `RocksStorageBundle` — backed by RocksDB `TransactionDB`
- `RawDiskDataStore` — flat-file block storage for file data
- All tests use RocksDB + `tempfile::tempdir()` for isolation

### Platform-Specific Code

- FUSE support (`client/src/fuse.rs`) is gated on `#[cfg(target_os = "linux")]` and uses the `fuser` crate
- The `rpc` crate uses `tonic` for gRPC with TLS support

### Root Inode

Root directory is inode 1 (`storage/src/allocator.rs::ROOT_INODE`). MetadataServer auto-initializes it on construction if absent.

## Git Commit Standards

When executing a long-term plan or a long-term cyclical output, make regular Git commits after achieving certain project results.

### 1. Commit Message Convention

All commits **must follow the Conventional Commits specification**.

#### Required Format

```
<type>(<scope>): <short description>
```

Example:

```
feat(server): binary skeleton for rpc serve
```

#### Allowed Commit Types

| Type | Meaning |
|------|--------|
| feat | A new feature |
| fix | A bug fix |
| refactor | Code change that neither fixes a bug nor adds a feature |
| perf | Performance improvement |
| docs | Documentation only changes |
| test | Adding or updating tests |
| chore | Maintenance work (build scripts, tooling, dependencies) |
| ci | CI/CD related changes |
| style | Formatting changes (no logic changes) |

#### Scope Rules

- Scope should describe the affected module or subsystem
- Use lowercase and short identifiers
- Prefer explicit module names
- Avoid empty scope unless absolutely necessary

Examples:

```
feat(server): add rpc handler
fix(storage): prevent race condition
refactor(fuse): simplify mount logic
feat(core): introduce shared config layer
```

#### Description Rules

The short description must:

- Be concise (≤ 72 characters)
- Use imperative mood
- Start with lowercase
- Not end with a period

Good:

```
feat(server): add streaming rpc support
```

Bad:

```
feat(server): Added streaming RPC support.
```

### 2. Commit Behavior Rules

#### Atomic Commits

Each commit must represent **a single logical change**.

Avoid:

- Mixing refactors and features
- Bundling unrelated changes

#### Prefer Small Commits

- Break large changes into incremental commits
- Each commit should compile and pass tests

#### No Broken States

The agent must:

- Ensure the project builds successfully
- Not commit failing tests
- Avoid temporary debugging artifacts

### 3. Code Modification Rules

#### Minimal Changes

- Modify only files necessary for the task
- Avoid unrelated formatting or style-only edits

#### Preserve Style

- Follow existing project conventions
- Match indentation and formatting

#### Backward Compatibility

Unless explicitly instructed:

- Do not introduce breaking API changes
- Preserve existing behavior

### 4. Safety Constraints

The agent must not:

- Delete critical files without explicit instruction
- Rewrite project history
- Change licensing information
- Reveal secrets or credentials

### 5. Documentation Expectations

For non-trivial changes, the agent should:

- Update relevant documentation
- Add inline comments where logic is complex
- Provide clear commit messages explaining intent

### 6. Example Good Commits

```
feat(server): add basic rpc server bootstrap
fix(storage): handle empty metadata snapshot
refactor(fuse): extract mount helper utilities
test(server): add rpc integration tests
docs(readme): describe server startup flow
```
