# rucksfs Technical Design Document

> **Version:** 0.2.0-draft
> **Last Updated:** 2026-02-13
> **Target Audience:** Developers (human & AI Agent) implementing rucksfs

---

## Table of Contents

- [rucksfs Technical Design Document](#rucksfs-technical-design-document)
  - [Table of Contents](#table-of-contents)
  - [1. Overview](#1-overview)
    - [1.1 Project Summary](#11-project-summary)
    - [1.2 Core Design Goals](#12-core-design-goals)
    - [1.3 How to Read This Document](#13-how-to-read-this-document)
  - [2. System Architecture](#2-system-architecture)
    - [2.1 Layered Architecture Overview](#21-layered-architecture-overview)
    - [2.2 Crate Dependency Graph](#22-crate-dependency-graph)
    - [2.3 gRPC Communication Protocol](#23-grpc-communication-protocol)
    - [2.4 Storage Module Decoupling](#24-storage-module-decoupling)
    - [2.5 Deployment Modes](#25-deployment-modes)
      - [Mode A: Distributed Deployment (Production)](#mode-a-distributed-deployment-production)
      - [Mode B: Single-Binary Demo (Development / Testing)](#mode-b-single-binary-demo-development--testing)
  - [3. Data Model \& Key-Value Encoding](#3-data-model--key-value-encoding)
    - [3.1 Core Types (from `core/src/lib.rs`)](#31-core-types-from-coresrclibrs)
    - [3.2 RocksDB Column Family Design](#32-rocksdb-column-family-design)
    - [3.3 Key Encoding Rules](#33-key-encoding-rules)
      - [`inodes` CF — Key](#inodes-cf--key)
      - [`dir_entries` CF — Key](#dir_entries-cf--key)
      - [`dir_entries` CF — Value](#dir_entries-cf--value)
      - [`system` CF — Key/Value](#system-cf--keyvalue)
      - [`delta_entries` CF — Key](#delta_entries-cf--key)
      - [`delta_entries` CF — Value](#delta_entries-cf--value)
    - [3.4 Value Serialization: `InodeValue`](#34-value-serialization-inodevalue)
    - [3.5 Encoding Summary Diagram](#35-encoding-summary-diagram)
  - [4. Inode Allocation \& Reclamation](#4-inode-allocation--reclamation)
    - [4.1 Reserved Inodes](#41-reserved-inodes)
    - [4.2 Allocation Strategy: Persistent Monotonic Counter](#42-allocation-strategy-persistent-monotonic-counter)
      - [Data Flow](#data-flow)
      - [Implementation](#implementation)
    - [4.3 Reclamation Strategy (Current Phase)](#43-reclamation-strategy-current-phase)
  - [5. Storage Engine Design](#5-storage-engine-design)
    - [5.1 RocksDB Metadata Engine](#51-rocksdb-metadata-engine)
      - [5.1.1 `RocksMetadataStore` — Implementing `MetadataStore`](#511-rocksmetadatastore--implementing-metadatastore)
      - [5.1.2 `RocksDirectoryIndex` — Implementing `DirectoryIndex`](#512-rocksdirectoryindex--implementing-directoryindex)
      - [5.1.3 Sharing a Single RocksDB Instance](#513-sharing-a-single-rocksdb-instance)
    - [5.2 Raw Disk Content Engine (`RawDiskDataStore`)](#52-raw-disk-content-engine-rawdiskdatastore)
      - [5.2.1 Design Overview](#521-design-overview)
      - [5.2.2 Implementation](#522-implementation)
      - [5.2.3 Inode ID as the Sole Link](#523-inode-id-as-the-sole-link)
    - [5.3 Module Replaceability](#53-module-replaceability)
    - [5.4 RocksDB Recommended Configuration](#54-rocksdb-recommended-configuration)
    - [5.5 Delta Entries \& Append-Only Write Path](#55-delta-entries--append-only-write-path)
      - [5.5.1 Motivation](#551-motivation)
      - [5.5.2 Architecture Overview](#552-architecture-overview)
      - [5.5.3 DeltaOp Enum (`server/src/delta.rs`)](#553-deltaop-enum-serversrcdeltars)
      - [5.5.4 Fold Semantics](#554-fold-semantics)
      - [5.5.5 DeltaStore Trait (`storage/src/lib.rs`)](#555-deltastore-trait-storagesrclibrs)
      - [5.5.6 InodeFoldedCache (`server/src/cache.rs`)](#556-inodefoldedcache-serversrccachers)
      - [5.5.7 Write Path: `append_parent_deltas`](#557-write-path-append_parent_deltas)
      - [5.5.8 Read Path: `load_inode`](#558-read-path-load_inode)
      - [5.5.9 Background Compaction Worker (`server/src/compaction.rs`)](#559-background-compaction-worker-serversrccompactionrs)
  - [6. POSIX Operations — Detailed Design](#6-posix-operations--detailed-design)
    - [Common Error Mapping](#common-error-mapping)
    - [6.1 Metadata Operations](#61-metadata-operations)
      - [6.1.1 `lookup`](#611-lookup)
      - [6.1.2 `getattr`](#612-getattr)
      - [6.1.3 `setattr`](#613-setattr)
      - [6.1.4 `statfs`](#614-statfs)
    - [6.2 Directory Operations](#62-directory-operations)
      - [6.2.1 `readdir`](#621-readdir)
      - [6.2.2 `create`](#622-create)
      - [6.2.3 `mkdir`](#623-mkdir)
      - [6.2.4 `unlink`](#624-unlink)
      - [6.2.5 `rmdir`](#625-rmdir)
      - [6.2.6 `rename`](#626-rename)
    - [6.3 Data Operations](#63-data-operations)
      - [6.3.1 `open`](#631-open)
      - [6.3.2 `read`](#632-read)
      - [6.3.3 `write`](#633-write)
      - [6.3.4 `flush`](#634-flush)
      - [6.3.5 `fsync`](#635-fsync)
  - [7. Transactions \& Consistency Guarantees](#7-transactions--consistency-guarantees)
    - [7.1 RocksDB WriteBatch Atomicity](#71-rocksdb-writebatch-atomicity)
    - [7.2 Operations Requiring Atomicity](#72-operations-requiring-atomicity)
    - [7.3 Concurrency Control Strategy](#73-concurrency-control-strategy)
      - [Strategy: PCC TransactionDB + execute_with_retry](#strategy-pcc-transactiondb--execute_with_retry)
    - [7.4 TOCTOU Prevention](#74-toctou-prevention)
  - [8. Security Mechanisms](#8-security-mechanisms)
    - [8.1 POSIX Permission Model](#81-posix-permission-model)
    - [8.2 RPC Authentication Integration](#82-rpc-authentication-integration)
    - [8.3 Data Integrity](#83-data-integrity)
  - [9. Fault Tolerance \& Crash Recovery](#9-fault-tolerance--crash-recovery)
    - [9.1 Failure Scenarios \& Expected Behavior](#91-failure-scenarios--expected-behavior)
    - [9.2 RocksDB WAL Crash Consistency](#92-rocksdb-wal-crash-consistency)
    - [9.3 RawDiskDataStore Recovery](#93-rawdiskdatastore-recovery)
    - [9.4 Inode Allocator Recovery](#94-inode-allocator-recovery)
  - [10. Configuration \& Tuning Recommendations](#10-configuration--tuning-recommendations)
    - [10.1 RocksDB Configuration Summary](#101-rocksdb-configuration-summary)
    - [10.2 RawDiskDataStore Configuration](#102-rawdiskdatastore-configuration)
    - [10.3 FUSE Mount Options](#103-fuse-mount-options)
    - [10.4 Performance Tuning Checklist](#104-performance-tuning-checklist)
  - [11. Glossary](#11-glossary)

---

## 1. Overview

### 1.1 Project Summary

rucksfs is a user-space file system implemented in Rust. It exposes a standard POSIX interface via Linux FUSE (`fuser` crate), with all storage logic handled by a pluggable server backend. The system is structured as a Cargo workspace containing six crates:

| Crate | Role |
|-------|------|
| `core` | Shared types (`FileAttr`, `DirEntry`, `StatFs`, `FsError`, `OpenResponse`, `DataLocation`) and trait definitions (`MetadataOps`, `DataOps`, `VfsOps`) |
| `storage` | Storage trait abstractions (`MetadataStore`, `DataStore`, `DirectoryIndex`, `DeltaStore`) and implementations (memory, RocksDB) |
| `server` | `MetadataServer` — namespace & attribute engine, implements `MetadataOps`, delegates data I/O to DataServer via `Arc<dyn DataOps>` |
| `dataserver` | `DataServer<D: DataStore>` — file data I/O engine, implements `DataOps` |
| `client` | `VfsCore` (routing), `EmbeddedClient` (in-process), FUSE adapter (`FuseClient`), `mount_fuse` |
| `rpc` | gRPC transport layer — `MetadataService` and `DataService` protobuf definitions, TLS, Bearer Token auth |
| `demo` | Single-binary assembly — embeds MetadataServer + DataServer + EmbeddedClient, bypasses gRPC for local testing |

### 1.2 Core Design Goals

1. **POSIX Compliance** — Implement all POSIX operations defined in the `MetadataOps` / `DataOps` / `VfsOps` traits with correct POSIX semantics (atomic rename, nlink maintenance, permission checks, etc.).
2. **Metadata / Data Separation** — Metadata (inode attributes, directory structure) and file content are stored in independent, pluggable engines linked only by inode ID.
3. **Module Decoupling** — Each storage module is defined by a trait (`MetadataStore`, `DataStore`, `DirectoryIndex`). Implementations can be swapped without changing upper-layer logic.
4. **Client / Server Separation** — The FUSE client and the storage server are independent components communicating via gRPC. They can be deployed on separate machines or compiled into a single binary for demo purposes.
5. **Crash Safety** — Leverage RocksDB WAL and WriteBatch for atomic metadata mutations; design content writes for idempotent recovery.
6. **Implementation-Ready** — Every section of this document provides enough detail (Rust pseudocode, byte-level encoding, error code mappings) for a developer or AI Agent to implement directly without external references.

### 1.3 How to Read This Document

- **Sections 2–5** define the architecture and data foundations. Read these first to understand the system's building blocks.
- **Section 6** is the core reference — it details every POSIX operation with interface signatures, step-by-step pseudocode, CF access patterns, and error mappings.
- **Sections 7–9** cover cross-cutting concerns (transactions, security, fault tolerance) that apply across all operations.
- **Section 10–11** provide operational guidance and terminology.
- All pseudocode uses Rust syntax and references the exact trait methods defined in the `core` and `storage` crates.

---

## 2. System Architecture

### 2.1 Layered Architecture Overview

rucksfs follows a strict **client / server** separation. The FUSE client handles VFS request reception; the storage server implements all POSIX semantics and data management. They communicate via gRPC (protobuf, defined in `rpc/proto/fuse.proto`).

```
┌─────────────────────────────────────────────────┐
│                 User Application                │
│              (ls, cat, cp, mv, ...)             │
└────────────────────┬────────────────────────────┘
                     │  POSIX syscalls
                     ▼
┌─────────────────────────────────────────────────┐
│              Linux VFS / FUSE Kernel            │
└────────────────────┬────────────────────────────┘
                     │  /dev/fuse
                     ▼
┌─────────────────────────────────────────────────┐
│           client crate (VfsCore + FuseClient)   │
│  ┌──────────────────────────────────────────┐   │
│  │  fuser::Filesystem impl (FuseClient)     │   │
│  │  Translates FUSE requests → VfsOps       │   │
│  └──────────────────┬───────────────────────┘   │
│                     │ VfsOps trait               │
│  ┌──────────────────▼───────────────────────┐   │
│  │  EmbeddedClient / RucksClient (gRPC)     │   │
│  │  Routes via VfsCore                      │   │
│  └────────┬─────────────────┬───────────────┘   │
└───────────┼─────────────────┼───────────────────┘
            │ MetadataOps     │ DataOps
            ▼                 ▼
┌────────────────────┐  ┌─────────────────────────┐
│ server crate       │  │ dataserver crate        │
│ (MetadataServer)   │  │ (DataServer<D>)         │
│ ┌────────────────┐ │  │ ┌───────────────────┐   │
│ │MetadataStore   │ │  │ │DataStore          │   │
│ │DirectoryIndex  │ │  │ │(Memory / RawDisk) │   │
│ │DeltaStore      │ │  │ └─────────┬─────────┘   │
│ └──────┬─────────┘ │  └───────────┼─────────────┘
└────────┼───────────┘              │
         │                          │
         ▼                          ▼
   ┌──────────────┐           ┌──────────┐
   │ RocksDB /    │           │ Raw Disk │
   │ Memory       │           │ / Memory │
   └──────────────┘           └──────────┘
```

### 2.2 Crate Dependency Graph

Dependencies flow **downward only** — no circular dependencies exist.

```
                    ┌──────┐
                    │ core │  (types + traits: MetadataOps, DataOps, VfsOps)
                    └──┬───┘
            ┌──────────┼──────────────┐
            ▼          ▼              ▼
       ┌─────────┐ ┌──────┐     ┌─────────┐
       │ storage │ │ rpc  │     │         │
       │         │ │      │     │         │
       └────┬────┘ └──┬───┘     │         │
            │         │         │         │
            ▼         │         │         │
       ┌─────────┐    │         │         │
       │ server  │────┘         │         │
       │         │              │         │
       └────┬────┘              │         │
            │                   │         │
            ▼                   ▼         │
       ┌──────────────────────────────┐   │
       │           client             │   │
       │  (depends: core, rpc)        │   │
       └──────────────┬───────────────┘   │
                      │                   │
                      ▼                   │
                 ┌─────────┐              │
                 │  demo   │──────────────┘
                 │  (all)  │
                 └─────────┘
```

Precise dependency edges per crate:

| Crate | Direct Dependencies |
|-------|-------------------|
| `core` | *(none — leaf crate)* |
| `storage` | `core` |
| `rpc` | `core` |
| `server` | `core`, `storage`, `rpc` |
| `client` | `core`, `rpc` |
| `demo` | `core`, `storage`, `server`, `client` |

### 2.3 gRPC Communication Protocol

The `rpc` crate defines two gRPC services — `MetadataService` (in `metadata.proto`) and `DataService` (in `data.proto`). The metadata service exposes namespace operations corresponding to `MetadataOps`, while the data service exposes I/O operations corresponding to `DataOps`. The RPC methods map as follows:

| RPC Method | Request Type | Response Type |
|-----------|-------------|--------------|
| `Lookup` | `LookupRequest(parent, name)` | `FileAttr` |
| `Getattr` | `GetattrRequest(inode)` | `FileAttr` |
| `Readdir` | `ReaddirRequest(inode)` | `ReaddirResponse(entries[])` |
| `Open` | `OpenRequest(inode, flags)` | `OpenResponse(handle)` |
| `Read` | `ReadRequest(inode, offset, size)` | `ReadResponse(data)` |
| `Write` | `WriteRequest(inode, offset, data, flags)` | `WriteResponse(bytes_written)` |
| `Create` | `CreateRequest(parent, name, mode)` | `FileAttr` |
| `Mkdir` | `MkdirRequest(parent, name, mode)` | `FileAttr` |
| `Unlink` | `UnlinkRequest(parent, name)` | `EmptyResponse` |
| `Rmdir` | `RmdirRequest(parent, name)` | `EmptyResponse` |
| `Rename` | `RenameRequest(parent, name, new_parent, new_name)` | `EmptyResponse` |
| `Setattr` | `SetattrRequest(inode, attr)` | `FileAttr` |
| `Statfs` | `StatfsRequest(inode)` | `StatFs` |
| `Flush` | `FlushRequest(inode)` | `EmptyResponse` |
| `Fsync` | `FsyncRequest(inode, datasync)` | `EmptyResponse` |

Transport security: TLS (via `tokio-rustls`) + Bearer Token authentication (constant-time comparison in `rpc/src/auth.rs`).

### 2.4 Storage Module Decoupling

The server's `MetadataServer<M, I, DS>` is generic over three independently replaceable storage backends, plus a trait-object reference to `DataOps` for delegating data I/O to a separate `DataServer`:

```rust
pub struct MetadataServer<M, I, DS>
where
    M: MetadataStore,   // inode attribute CRUD
    I: DirectoryIndex,  // directory structure
    DS: DeltaStore,     // append-only delta entries for inode attributes
{
    pub metadata: Arc<M>,
    pub index: Arc<I>,
    pub delta_store: Arc<DS>,
    /// Client for talking to the DataServer (for truncate/delete on
    /// setattr size change or unlink with nlink=0).
    pub data_client: Arc<dyn DataOps>,
    /// DataServer endpoint info returned in OpenResponse.
    pub data_location: DataLocation,
    /// LRU cache of folded inode values.
    pub cache: Arc<InodeFoldedCache>,
    /// Background compaction worker (shared with the MetadataServer).
    pub compaction: Arc<DeltaCompactionWorker<M, DS>>,
    allocator: InodeAllocator,
    /// Storage bundle for atomic cross-store writes.
    storage_bundle: Arc<dyn StorageBundle>,
}
```

**Why `Arc<dyn DataOps>` instead of a generic `D: DataStore`?** The `MetadataServer` only needs to call `DataOps::truncate` and `DataOps::delete_data` during `setattr` (size change) and `unlink` (nlink=0). Using a trait object keeps the generic parameter list concise and aligns with the architectural goal of metadata/data separation — the `MetadataServer` talks to a `DataServer` through the `DataOps` interface, not directly to a `DataStore`.

**Key decoupling principle:** `MetadataStore` and `DataStore` share **no direct dependency**. They are linked solely by **inode ID** — the metadata engine stores inode attributes keyed by inode ID, and the data engine reads/writes content keyed by the same inode ID. Neither engine needs to know the other's implementation.

The `DeltaStore` is an **append-only** log of incremental attribute modifications (nlink changes, timestamp updates). On the write path, directory operations append deltas instead of doing a full read-modify-write of the parent inode. On the read path, deltas are folded on top of the base inode value to produce the current state (see §5.5 for details).

| Trait | Current Implementation | Future Alternatives |
|-------|----------------------|-------------------|
| `MetadataStore` | RocksDB | SQLite, TiKV, etcd |
| `DataStore` | `RawDiskDataStore` (local raw file) | S3, Ceph RADOS, MinIO |
| `DirectoryIndex` | RocksDB (same instance as MetadataStore) | In-memory trie, Redis |
| `DeltaStore` | RocksDB (`delta_entries` CF) / `MemoryDeltaStore` | Redis Streams, Kafka |

### 2.5 Deployment Modes

#### Mode A: Distributed Deployment (Production)

Client and server run as separate processes, potentially on different machines:

```
┌─────────────┐      gRPC/TLS       ┌─────────────────┐
│ Machine A   │ ◄──────────────────► │ Machine B       │
│  client bin │                      │  server bin     │
│  FUSE mount │                      │  RocksDB +      │
│             │                      │  data.img       │
└─────────────┘                      └─────────────────┘
```

- `client/src/main.rs` — starts FUSE mount, connects to remote server via gRPC.
- `server/src/main.rs` — starts gRPC server, instantiates storage backends.

#### Mode B: Single-Binary Demo (Development / Testing)

The `demo` crate compiles MetadataServer, DataServer, and EmbeddedClient into one process, **bypassing gRPC entirely**. The servers are injected directly into the client's VFS layer via `Arc<dyn MetadataOps>` and `Arc<dyn DataOps>`.

```rust
// demo/src/main.rs — assembly sequence
let metadata_store = Arc::new(MemoryMetadataStore::new());
let dir_index      = Arc::new(MemoryDirectoryIndex::new());
let delta_store    = Arc::new(MemoryDeltaStore::new());
let data_store     = Arc::new(MemoryDataStore::new());

let data_server = Arc::new(DataServer::new(data_store));
// data_server implements DataOps

let meta_server = Arc::new(MetadataServer::new(
    metadata_store, dir_index, delta_store, data_server.clone(),
));
// meta_server implements MetadataOps

let client = EmbeddedClient::new(meta_server, data_server);
// client implements VfsOps, routes metadata → MetadataOps, data → DataOps
```

**Injection chain:** `Concrete Storage Impls` → `DataServer<D>` (implements `DataOps`) + `MetadataServer` (implements `MetadataOps`) → `EmbeddedClient` (implements `VfsOps`) → `FuseClient` (implements `fuser::Filesystem`)

This demo mode is the primary development target. All design decisions in this document are implementable within this single-binary assembly.

---

## 3. Data Model & Key-Value Encoding

### 3.1 Core Types (from `core/src/lib.rs`)

The following types are the single source of truth. All encoding/decoding must match these exactly.

```rust
pub type Inode = u64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileAttr {
    pub inode: Inode,  // unique identifier, links metadata ↔ data
    pub size:  u64,    // file content length in bytes
    pub mode:  u32,    // POSIX permission bits + file type (S_IFDIR | S_IFREG)
    pub uid:   u32,    // owner user ID
    pub gid:   u32,    // owner group ID
    pub atime: u64,    // last access time (Unix timestamp, seconds)
    pub mtime: u64,    // last modification time
    pub ctime: u64,    // last status change time
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DirEntry {
    pub name:  String, // child entry name (UTF-8, max 255 bytes)
    pub inode: Inode,  // child inode number
    pub kind:  u32,    // file type constant (e.g. S_IFREG=0o100000, S_IFDIR=0o040000)
}
```

**Design note — `nlink` field:** The current `FileAttr` struct does not include `nlink` (hard link count). This field will be **added** when implementing create/mkdir/unlink/rmdir. It must be stored as `u32` and serialized as part of the inode value. Until it is added to the Rust struct, the KV encoding reserves 4 bytes for it at a fixed offset (see §3.3 below). Initial values: regular file = 1, directory = 2 (for `.` and `..`).

### 3.2 RocksDB Column Family Design

All metadata is stored in a single RocksDB instance with three Column Families (CFs). Using separate CFs enables independent compaction, bloom filter tuning, and atomic cross-CF writes via `WriteBatch`.

| CF Name | Purpose | Key Format | Value Format |
|---------|---------|------------|--------------|
| `inodes` | Inode attributes (base values) | `[b'I'][inode_id: u64 BE]` (9 bytes) | Serialized `InodeValue` (see §3.3) |
| `dir_entries` | Directory children | `[b'D'][parent_inode: u64 BE][child_name: UTF-8]` (variable) | `child_inode` (8 BE bytes) + `child_kind` (4 BE bytes) |
| `delta_entries` | Append-only inode attribute deltas | `[b'X'][inode: u64 BE][seq: u64 BE]` (17 bytes) | Serialized `DeltaOp` (5–9 bytes, see §5.5) |
| `system` | System-level counters | ASCII key string (e.g. `b"next_inode"`) | Value depends on key (e.g. 8 BE bytes for counters) |

**Why four CFs instead of one?**
- `inodes` CF has point-lookup access pattern (get by inode) → optimize with bloom filter.
- `dir_entries` CF has prefix-scan access pattern (list children of a parent) → optimize with prefix extractor.
- `delta_entries` CF has append-heavy, prefix-scan access pattern (scan all deltas for an inode) → optimize with prefix extractor on first 9 bytes (`[b'X'][inode]`). Separated from `inodes` to avoid write amplification during RocksDB compaction of hot base values.
- `system` CF is rarely accessed, very small → separate compaction avoids interference.

### 3.3 Key Encoding Rules

All multi-byte integer fields use **big-endian** byte order. This ensures that RocksDB's default lexicographic byte comparison produces numerical ordering, which is critical for range scans and prefix iteration.

#### `inodes` CF — Key

```
┌──────────────────────┐
│  inode_id: u64 (BE)  │   8 bytes, fixed length
└──────────────────────┘
```

Encoding (Rust):
```rust
fn encode_inode_key(inode: Inode) -> [u8; 8] {
    inode.to_be_bytes()
}
```

#### `dir_entries` CF — Key

```
┌──────────────────────┬────────────────────────┐
│ parent_inode: u64 BE │  child_name: UTF-8     │
│       8 bytes        │  variable (1-255 bytes)│
└──────────────────────┴────────────────────────┘
```

No separator is needed because the parent_inode field is fixed-length (8 bytes). The child_name starts at offset 8.

Encoding (Rust):
```rust
fn encode_dir_key(parent: Inode, name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(8 + name.len());
    key.extend_from_slice(&parent.to_be_bytes());
    key.extend_from_slice(name.as_bytes());
    key
}

fn decode_dir_key(key: &[u8]) -> (Inode, &str) {
    let parent = u64::from_be_bytes(key[..8].try_into().unwrap());
    let name = std::str::from_utf8(&key[8..]).unwrap();
    (parent, name)
}
```

#### `dir_entries` CF — Value

```
┌──────────────────────┬──────────────────┐
│ child_inode: u64 BE  │ child_kind: u32  │
│       8 bytes        │    4 bytes (BE)  │
└──────────────────────┴──────────────────┘
```

Total: 12 bytes, fixed length.

#### `system` CF — Key/Value

| Key (ASCII bytes) | Value | Description |
|---|---|---|
| `b"next_inode"` | `u64` (8 BE bytes) | Next inode ID to allocate |
| `b"fs_stats"` | Serialized `StatFs` | Cached filesystem statistics |

#### `delta_entries` CF — Key

```
┌───────────────────┬──────────────────────┬──────────────────────┐
│ prefix: u8 = 'X'  │  inode: u64 (BE)     │  seq: u64 (BE)       │
│       1 byte       │       8 bytes        │       8 bytes        │
└───────────────────┴──────────────────────┴──────────────────────┘
```

Total: 17 bytes, fixed length. The prefix byte `b'X'` distinguishes delta keys from inode (`b'I'`) and dir-entry (`b'D'`) keys. Keys with the same inode are ordered by `seq` (big-endian guarantees lexicographic = numerical order).

Encoding (Rust):
```rust
const DELTA_KEY_PREFIX: u8 = b'X';

fn encode_delta_key(inode: Inode, seq: u64) -> [u8; 17] {
    let mut key = [0u8; 17];
    key[0] = DELTA_KEY_PREFIX;
    key[1..9].copy_from_slice(&inode.to_be_bytes());
    key[9..17].copy_from_slice(&seq.to_be_bytes());
    key
}

fn decode_delta_key(key: &[u8]) -> FsResult<(Inode, u64)> {
    // key[0] == b'X', key[1..9] = inode, key[9..17] = seq
    let inode = u64::from_be_bytes(key[1..9].try_into().unwrap());
    let seq   = u64::from_be_bytes(key[9..17].try_into().unwrap());
    Ok((inode, seq))
}

/// Prefix for scanning all deltas of a given inode: [b'X'][inode: u64 BE] (9 bytes).
fn delta_prefix(inode: Inode) -> [u8; 9] {
    let mut prefix = [0u8; 9];
    prefix[0] = DELTA_KEY_PREFIX;
    prefix[1..9].copy_from_slice(&inode.to_be_bytes());
    prefix
}
```

#### `delta_entries` CF — Value

Each value is a single serialized `DeltaOp`: a 1-byte op-type tag followed by a fixed-size payload.

```
┌──────────────────┬──────────────────────────────┐
│ op_type: u8      │  payload (BE)                │
│    1 byte        │  4 bytes (i32) or 8 bytes    │
└──────────────────┴──────────────────────────────┘
```

| Op Type Tag | Payload | Total Size | Meaning |
|---|---|---|---|
| `1` (`OP_INCREMENT_NLINK`) | `i32` (4 BE bytes) | 5 bytes | Add signed delta to `nlink` |
| `2` (`OP_SET_MTIME`) | `u64` (8 BE bytes) | 9 bytes | Set `mtime` (fold takes max) |
| `3` (`OP_SET_CTIME`) | `u64` (8 BE bytes) | 9 bytes | Set `ctime` (fold takes max) |
| `4` (`OP_SET_ATIME`) | `u64` (8 BE bytes) | 9 bytes | Set `atime` (fold takes max) |

**Design rationale:** Keeping each delta as a small, self-contained blob (5–9 bytes) makes append cheap and sequential scan efficient. The op-type tag enables forward-compatible extension — new delta types can be added without breaking existing entries.

### 3.4 Value Serialization: `InodeValue`

The `inodes` CF value stores a versioned binary structure called `InodeValue`. Serialization uses **hand-crafted big-endian binary encoding** (deterministic, fixed-size, no external dependency). Each field is written as a fixed-width big-endian integer, yielding a 57-byte record.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InodeValue {
    pub version: u8,       // schema version, currently = 1
    // --- FileAttr fields ---
    pub inode: u64,
    pub size: u64,
    pub mode: u32,
    pub nlink: u32,        // hard link count
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}
```

**Version compatibility strategy:**
- The first byte is always the schema version.
- When deserializing, check the version byte first. If `version > CURRENT_VERSION`, return `FsError::InvalidInput`.
- Adding new fields at the end is a forward-compatible change (bump version, older readers skip unknown trailing bytes).

**Serialization layout (57 bytes):**

```
Offset   Field     Size   Encoding
──────   ─────     ────   ────────
 0       version    1B    u8
 1       inode      8B    u64 BE
 9       size       8B    u64 BE
17       mode       4B    u32 BE
21       nlink      4B    u32 BE
25       uid        4B    u32 BE
29       gid        4B    u32 BE
33       atime      8B    u64 BE
41       mtime      8B    u64 BE
49       ctime      8B    u64 BE
                   ────
Total:             57 bytes
```

**Why hand-crafted encoding instead of bincode?** Hand-crafted big-endian encoding guarantees deterministic byte layout across Rust compiler versions and architectures. `bincode` uses variable-length integer encoding by default, which can produce different byte sequences depending on configuration. The fixed 57-byte layout also simplifies debugging and hex inspection.

Conversion helpers:
```rust
impl InodeValue {
    const CURRENT_VERSION: u8 = 1;
    const V1_LEN: usize = 57;

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::V1_LEN);
        buf.push(self.version);
        buf.extend_from_slice(&self.inode.to_be_bytes());
        buf.extend_from_slice(&self.size.to_be_bytes());
        buf.extend_from_slice(&self.mode.to_be_bytes());
        buf.extend_from_slice(&self.nlink.to_be_bytes());
        buf.extend_from_slice(&self.uid.to_be_bytes());
        buf.extend_from_slice(&self.gid.to_be_bytes());
        buf.extend_from_slice(&self.atime.to_be_bytes());
        buf.extend_from_slice(&self.mtime.to_be_bytes());
        buf.extend_from_slice(&self.ctime.to_be_bytes());
        buf
    }

    pub fn deserialize(data: &[u8]) -> FsResult<Self> {
        if data.is_empty() {
            return Err(FsError::InvalidInput("empty inode value".into()));
        }
        let version = data[0];
        if version != Self::CURRENT_VERSION {
            return Err(FsError::InvalidInput(
                format!("unsupported inode version {}", version)));
        }
        if data.len() < Self::V1_LEN {
            return Err(FsError::InvalidInput("inode value too short".into()));
        }
        Ok(Self {
            version,
            inode: u64::from_be_bytes(data[1..9].try_into().unwrap()),
            size:  u64::from_be_bytes(data[9..17].try_into().unwrap()),
            mode:  u32::from_be_bytes(data[17..21].try_into().unwrap()),
            nlink: u32::from_be_bytes(data[21..25].try_into().unwrap()),
            uid:   u32::from_be_bytes(data[25..29].try_into().unwrap()),
            gid:   u32::from_be_bytes(data[29..33].try_into().unwrap()),
            atime: u64::from_be_bytes(data[33..41].try_into().unwrap()),
            mtime: u64::from_be_bytes(data[41..49].try_into().unwrap()),
            ctime: u64::from_be_bytes(data[49..57].try_into().unwrap()),
        })
    }

    pub fn to_attr(&self) -> FileAttr {
        FileAttr {
            inode: self.inode, size: self.size, mode: self.mode,
            nlink: self.nlink, uid: self.uid, gid: self.gid,
            atime: self.atime, mtime: self.mtime, ctime: self.ctime,
        }
    }
}
```

### 3.5 Encoding Summary Diagram

```
  inodes CF:
    Key:   [  inode u64 BE  ]  →  Value: [ ver | inode | size | mode | nlink | uid | gid | atime | mtime | ctime ]
                                           1B     8B     8B    4B     4B     4B    4B    8B      8B      8B
                                           Total value: 57 bytes (hand-crafted BE encoding)

  dir_entries CF:
    Key:   [ parent_inode u64 BE | child_name UTF-8 ]  →  Value: [ child_inode u64 BE | kind u32 BE ]
            8 bytes                1-255 bytes                     8 bytes              4 bytes

  delta_entries CF:
    Key:   [ b'X' | inode u64 BE | seq u64 BE ]  →  Value: [ op_type u8 | payload (4 or 8 bytes) ]
             1B      8 bytes       8 bytes                    1B           variable
             Total key: 17 bytes                              Total value: 5 bytes (nlink) or 9 bytes (timestamps)

  system CF:
    Key:   [ ASCII string ]  →  Value: [ varies ]
```

---

## 4. Inode Allocation & Reclamation

### 4.1 Reserved Inodes

| Inode | Purpose |
|-------|--------|
| 0 | Reserved — never allocated. Some FUSE/VFS code uses 0 as "invalid inode". |
| 1 | Root directory (`/`). This is a FUSE convention — `fuser` calls `lookup` with `parent=1` for the mount root. |

The root directory inode (1) is created during filesystem initialization (`mkfs` / first boot). Its initial `FileAttr`:

```rust
FileAttr {
    inode: 1,
    size: 0,
    mode: 0o040755,    // S_IFDIR | rwxr-xr-x
    uid: 0, gid: 0,   // owned by root
    atime: now, mtime: now, ctime: now,
}
// nlink = 2 (self "." + parent ".." pointing to itself)
```

### 4.2 Allocation Strategy: Persistent Monotonic Counter

Inode IDs are allocated using a **monotonically increasing counter** stored in the `system` CF and cached in memory.

**Why this approach?**
- Simplicity: no fragmentation, no free-list management.
- Thread safety: `AtomicU64` for lock-free allocation.
- Crash safety: persistent counter in RocksDB survives restarts.
- 64-bit space: 2^64 inodes is practically inexhaustible (at 1 million allocations/sec, lasts 584,942 years).

#### Data Flow

```
┌─────────────────────────┐     on startup      ┌──────────────────────────┐
│  system CF              │ ──────────────────►  │  InodeAllocator          │
│  key: "next_inode"      │                      │  next: AtomicU64         │
│  value: 42 (persisted)  │                      │  next.load() → 42       │
└─────────────────────────┘                      └──────────────────────────┘
                                                          │
                                                   alloc() called
                                                          │
                                                   fetch_add(1) → returns 42
                                                   (next is now 43)
                                                          │
                                          periodically or on batch commit:
                                          persist next=43 to system CF
```

#### Implementation

```rust
pub struct InodeAllocator {
    next: AtomicU64,
}

impl InodeAllocator {
    /// Load persisted counter from system CF on startup.
    /// If key "next_inode" does not exist (first boot), initialize to 2
    /// (inodes 0 and 1 are reserved).
    pub fn open(metadata: &impl MetadataStore) -> FsResult<Self> {
        let key = b"next_inode";
        let next = match metadata.get(key)? {
            Some(bytes) => u64::from_be_bytes(bytes.try_into().map_err(|_|
                FsError::InvalidInput("corrupt next_inode counter".into()))?),
            None => {
                // First boot: persist initial value
                metadata.put(key, &2u64.to_be_bytes())?;
                2
            }
        };
        Ok(Self { next: AtomicU64::new(next) })
    }

    /// Allocate a new inode ID. Thread-safe, lock-free.
    pub fn alloc(&self) -> Inode {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Persist the current counter value to RocksDB.
    /// Called as part of the WriteBatch that creates the inode,
    /// ensuring atomicity: if the batch fails, the counter is not advanced on disk.
    pub fn persist(&self, metadata: &impl MetadataStore) -> FsResult<()> {
        let val = self.next.load(Ordering::Relaxed);
        metadata.put(b"next_inode", &val.to_be_bytes())
    }
}
```

**Atomicity guarantee:** The `persist()` call is included in the same `WriteBatch` as the new inode's metadata insertion. If the batch fails, the on-disk counter remains unchanged. On restart, `open()` reloads the old counter, and the in-memory `AtomicU64` may have advanced beyond the persisted value — but those "phantom" inodes were never committed to any CF, so they are harmlessly skipped.

### 4.3 Reclamation Strategy (Current Phase)

**Current design: no reclamation.** Deleted inodes are not recycled. The counter only moves forward.

**Rationale:**
- 64-bit counter space is effectively unlimited.
- Reclamation adds complexity (free lists, ABA problems, delayed cleanup).
- For the demo phase, simplicity is prioritized.

**Future extension path (not implemented now):**
1. **Deferred free list:** On `unlink`/`rmdir`, push the freed inode ID onto a persistent free list in the `system` CF (key: `b"free_inodes"`, value: packed array of u64).
2. **Allocation with free list:** `alloc()` first pops from the free list; if empty, falls back to the monotonic counter.
3. **Open-file guard:** If a file is still open when unlinked (nlink=0 but open handles > 0), defer the inode reclamation until the last handle is closed. Track open handles with an in-memory `HashMap<Inode, u32>` (see §6 `unlink` design).

---

## 5. Storage Engine Design

This section describes the concrete implementations of the three storage traits. Both metadata traits (`MetadataStore`, `DirectoryIndex`) share a single RocksDB instance with separate Column Families. The content trait (`DataStore`) uses an independent raw file engine.

### 5.1 RocksDB Metadata Engine

A single RocksDB instance manages all metadata, using the three CFs defined in §3.2 (`inodes`, `dir_entries`, `system`).

#### 5.1.1 `RocksMetadataStore` — Implementing `MetadataStore`

The `MetadataStore` trait provides a raw KV interface for the `inodes` CF **only**. Directory entries and system keys are handled by `RocksDirectoryIndex` and the `system` CF helper respectively. Each trait implementation is responsible for its own Column Family.

```rust
pub struct RocksMetadataStore {
    db: Arc<TransactionDB>,
}
```

**Trait method implementations (all operate on the `inodes` CF):**

| Trait Method | RocksDB Operation | Notes |
|---|---|---|
| `get(key)` | `db.get_cf(cf_inodes, key)` | Returns `Ok(None)` if key not found, not `FsError::NotFound` |
| `put(key, value)` | `db.put_cf(cf_inodes, key, value)` | Single-key write; for multi-key atomicity, see §7 |
| `delete(key)` | `db.delete_cf(cf_inodes, key)` | Idempotent — deleting a non-existent key is not an error |
| `scan_prefix(prefix)` | `db.prefix_iterator_cf(cf_inodes, prefix)` | Used by system-level counters (e.g. `next_inode`) |

**CF separation strategy:** Rather than routing keys to different CFs via a tag-byte prefix, each storage trait directly operates on its own CF:

| Trait | Owns CF | Description |
|---|---|---|
| `RocksMetadataStore` | `inodes` | Inode attribute base values |
| `RocksDirectoryIndex` | `dir_entries` | Directory children mappings |
| `RocksDeltaStore` | `delta_entries` | Append-only inode attribute deltas |
| (system keys) | `system` | Allocator counter, stats — accessed via raw `TransactionDB` methods |

This design is cleaner than tag-byte routing: each implementation knows exactly which CF it owns, and there is no key-prefix stripping logic.

#### 5.1.2 `RocksDirectoryIndex` — Implementing `DirectoryIndex`

The `DirectoryIndex` trait provides directory-specific operations built on top of the `dir_entries` CF.

```rust
pub struct RocksDirectoryIndex {
    db: Arc<rocksdb::DB>,   // same RocksDB instance as RocksMetadataStore
}
```

**Trait method implementations:**

| Method | Signature | Implementation |
|---|---|---|
| `resolve_path(parent, name)` | `→ FsResult<Option<Inode>>` | Point-lookup in `dir_entries` CF with key = `encode_dir_key(parent, name)`. Decode value to extract `child_inode`. Return `None` if key absent. |
| `list_dir(inode)` | `→ FsResult<Vec<DirEntry>>` | Prefix-scan `dir_entries` CF with prefix = `inode.to_be_bytes()` (8 bytes). For each (key, value), decode child name from key\[8..\] and (child\_inode, kind) from value. Returns only real children — does **not** synthesize `.` and `..` entries (the FUSE kernel module handles these automatically). |
| `insert_child(parent, name, child_inode, attr)` | `→ FsResult<()>` | Put to `dir_entries` CF: key = `encode_dir_key(parent, name)`, value = `child_inode.to_be_bytes() ++ kind.to_be_bytes()`. The `kind` is extracted from `attr.mode & S_IFMT`. |
| `remove_child(parent, name)` | `→ FsResult<()>` | Delete from `dir_entries` CF: key = `encode_dir_key(parent, name)`. |

**Pseudocode for `list_dir`:**

```rust
fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
    let prefix = inode.to_be_bytes();
    let cf = self.db.cf_handle("dir_entries").unwrap();
    let iter = self.db.prefix_iterator_cf(&cf, &prefix);

    let mut entries = Vec::new();
    // Note: "." and ".." are NOT synthesized here.
    // The FUSE kernel module injects them automatically.

    for item in iter {
        let (key, value) = item.map_err(|e| FsError::Io(e.to_string()))?;
        if !key.starts_with(&prefix) { break; }

        let child_name = std::str::from_utf8(&key[8..])
            .map_err(|_| FsError::InvalidInput("non-UTF8 filename".into()))?;
        let child_inode = u64::from_be_bytes(value[..8].try_into().unwrap());
        let kind = u32::from_be_bytes(value[8..12].try_into().unwrap());

        entries.push(DirEntry {
            name: child_name.to_string(),
            inode: child_inode,
            kind,
        });
    }
    Ok(entries)
}
```

#### 5.1.3 Sharing a Single RocksDB Instance

Both `RocksMetadataStore` and `RocksDirectoryIndex` hold `Arc<rocksdb::DB>` references to the **same** RocksDB instance. This is critical for `WriteBatch` atomicity — a single WriteBatch can include operations across all three CFs within the same DB instance.

```rust
// Initialization (in MetadataServer::new or demo assembly)
let db_opts = rocksdb::Options::default();
db_opts.create_if_missing(true);
db_opts.create_missing_column_families(true);

let cf_descriptors = vec![
    ColumnFamilyDescriptor::new("inodes", cf_opts_inodes()),
    ColumnFamilyDescriptor::new("dir_entries", cf_opts_dir()),
    ColumnFamilyDescriptor::new("system", Options::default()),
];

let db = Arc::new(DB::open_cf_descriptors(&db_opts, "/data/rucksfs_meta", cf_descriptors)?);

let metadata_store = RocksMetadataStore { db: db.clone(), ... };
let dir_index      = RocksDirectoryIndex { db: db.clone() };
```

### 5.2 Raw Disk Content Engine (`RawDiskDataStore`)

The content storage engine uses a **local flat file** (`data.img`) as a virtual raw disk. Each inode is assigned a contiguous region within this file, addressed by simple offset arithmetic.

#### 5.2.1 Design Overview

```
┌───────────────────────────────────────────────────────────┐
│                      data.img (Raw Disk)                  │
├──────────┬──────────┬──────────┬──────────┬──────────┬────┤
│  Inode 0 │  Inode 1 │  Inode 2 │  Inode 3 │  Inode 4 │...│
│ (unused) │ (root /) │ max_size │ max_size │ max_size │   │
│ max_size │ max_size │          │          │          │   │
├──────────┼──────────┼──────────┼──────────┼──────────┼────┤
│ Blocks:  │ Blocks:  │ Blocks:  │          │          │   │
│ 0..N-1   │ N..2N-1  │ 2N..3N-1 │          │          │   │
└──────────┴──────────┴──────────┴──────────┴──────────┴────┘
    Block size = 4 KiB
    max_file_size = per-inode capacity (e.g. 16 MiB = 4096 blocks)
```

**Offset formula:**
```
disk_offset = inode * max_file_size + file_offset
```

#### 5.2.2 Implementation

```rust
use std::os::unix::fs::FileExt; // provides read_at / write_at on File

pub struct RawDiskDataStore {
    file: Arc<std::fs::File>,   // opened with O_RDWR
    max_file_size: u64,         // per-inode capacity (default: 16 MiB)
}

impl RawDiskDataStore {
    pub fn open(path: &str, max_file_size: u64) -> FsResult<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true).write(true).create(true)
            .open(path)
            .map_err(|e| FsError::Io(e.to_string()))?;
        Ok(Self { file: Arc::new(file), max_file_size })
    }

    fn disk_offset(&self, inode: Inode, offset: u64) -> FsResult<u64> {
        if offset >= self.max_file_size {
            return Err(FsError::InvalidInput("offset exceeds max file size".into()));
        }
        Ok(inode * self.max_file_size + offset)
    }
}

#[async_trait]
impl DataStore for RawDiskDataStore {
    async fn read_at(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let disk_off = self.disk_offset(inode, offset)?;
        let clamped_size = std::cmp::min(
            size as u64,
            self.max_file_size - offset
        ) as usize;
        let mut buf = vec![0u8; clamped_size];
        let n = self.file.read_at(&mut buf, disk_off)
            .map_err(|e| FsError::Io(e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    async fn write_at(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32> {
        let disk_off = self.disk_offset(inode, offset)?;
        let max_write = (self.max_file_size - offset) as usize;
        let write_len = std::cmp::min(data.len(), max_write);
        let n = self.file.write_at(&data[..write_len], disk_off)
            .map_err(|e| FsError::Io(e.to_string()))?;
        Ok(n as u32)
    }

    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()> {
        // Zero-fill from [size, max_file_size) for this inode's region
        let disk_off = self.disk_offset(inode, size)?;
        let zero_len = (self.max_file_size - size) as usize;
        let zeros = vec![0u8; std::cmp::min(zero_len, 64 * 1024)];
        let mut remaining = zero_len;
        let mut off = disk_off;
        while remaining > 0 {
            let chunk = std::cmp::min(remaining, zeros.len());
            self.file.write_at(&zeros[..chunk], off)
                .map_err(|e| FsError::Io(e.to_string()))?;
            off += chunk as u64;
            remaining -= chunk;
        }
        Ok(())
    }

    async fn flush(&self, _inode: Inode) -> FsResult<()> {
        self.file.sync_data()
            .map_err(|e| FsError::Io(e.to_string()))
    }
}
```

**Key properties of `pread`/`pwrite` (via `FileExt`):**
- **Thread-safe**: Multiple threads can concurrently read/write different inode regions without locking, because `pread`/`pwrite` do not modify the file descriptor's seek position.
- **Atomic for small writes**: POSIX guarantees that `pwrite` to a regular file is atomic for writes ≤ `PIPE_BUF` (typically 4096 bytes). Larger writes may be non-atomic (see §9 fault tolerance).

#### 5.2.3 Inode ID as the Sole Link

The metadata engine and content engine share **no state** except the `Inode` type (a `u64`). The flow:

```
  MetadataServer.write(inode=5, offset=1024, data)
         │
         ├──► MetadataStore.get(inode_key(5))     → FileAttr (validate existence)
         │
         ├──► DataStore.write_at(inode=5, 1024, data)  → bytes_written
         │         └─ disk_offset = 5 * 16MiB + 1024
         │
         └──► MetadataStore.put(inode_key(5), updated_attr)  → update size/mtime
```

Neither engine holds a reference to the other. The `MetadataServer` orchestrates both through their respective trait interfaces.

### 5.3 Module Replaceability

The trait-based design ensures that any storage backend can be replaced without modifying `MetadataServer` or any upper-layer code:

| Replacement Scenario | What Changes | What Stays |
|---|---|---|
| RocksDB → SQLite for metadata | Implement `MetadataStore` + `DirectoryIndex` for SQLite | `MetadataServer`, `MetadataOps` logic, client, FUSE layer |
| Raw file → S3 for data | Implement `DataStore` for S3 | `DataServer`, `DataOps` logic, client, FUSE layer |
| Single-node → distributed | Add sharding in `MetadataStore` impl | `MetadataOps` / `DataOps` semantics remain identical |

The trait-based constraints enforce this at runtime via dynamic dispatch:
```rust
impl MetadataOps for MetadataServer {
    // MetadataServer holds Arc<dyn MetadataStore>, Arc<dyn DirectoryIndex>,
    // Arc<dyn DeltaStore>, Arc<dyn DataOps>
    ...
}

impl<D: DataStore> DataOps for DataServer<D> {
    // DataServer<D> holds D: DataStore
    ...
}
```

#### 5.3.1 `StorageBundle` & `AtomicWriteBatch` Abstraction

> **Not in original design.** The code introduces two additional traits that enable **cross-CF atomic writes** without coupling the server to a specific storage engine.

```rust
/// Operation types that can be collected into an atomic write batch.
pub enum BatchOp {
    PutInode { key: Vec<u8>, value: Vec<u8> },
    DeleteInode { key: Vec<u8> },
    PutDirEntry { key: Vec<u8>, value: Vec<u8> },
    DeleteDirEntry { key: Vec<u8> },
    PutDelta { key: Vec<u8>, value: Vec<u8> },
    DeleteDelta { key: Vec<u8> },
    PutSystem { key: Vec<u8>, value: Vec<u8> },
}

/// A batch of write operations committed atomically.
pub trait AtomicWriteBatch: Send {
    fn push(&mut self, op: BatchOp);
    fn commit(self: Box<Self>) -> FsResult<()>;
    /// PCC row lock on the inodes CF.
    fn get_for_update_inode(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>>;
    /// PCC row lock on the dir_entries CF.
    fn get_for_update_dir_entry(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>>;
}

/// Factory for atomic write batches — owns references to all storage backends.
pub trait StorageBundle: Send + Sync {
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_>;
}
```

The `RocksStorageBundle` implementation holds an `Arc<TransactionDB>` and creates a PCC `Transaction` on each `begin_write()`. The `MetadataServer` receives a `storage_bundle: Arc<dyn StorageBundle>` at construction time and uses it for all mutating operations.

**Why this abstraction?** It decouples the server's transaction logic from the concrete RocksDB API. A future `MemoryStorageBundle` (or `SqliteStorageBundle`) can implement the same traits for testing or alternative backends.

### 5.4 RocksDB Recommended Configuration

| Parameter | Recommended Value | Rationale |
|---|---|---|
| `write_buffer_size` | 64 MiB | Balance between write throughput and memory usage for metadata workloads |
| `max_write_buffer_number` | 3 | Allow 2 immutable memtables flushing while 1 is active |
| `target_file_size_base` | 64 MiB | Appropriate for small-value metadata records |
| `max_background_compactions` | 4 | Utilize multi-core for compaction parallelism |
| `bloom_filter_bits_per_key` | 10 | ~1% false positive rate; apply to `inodes` CF for point lookups |
| `prefix_extractor` | `FixedPrefix(8)` | For `dir_entries` CF — first 8 bytes = parent inode, enables efficient prefix scan |
| `block_cache_size` | 128 MiB | Cache hot inode blocks; shared across all CFs |
| `compression` | LZ4 (L0-L1), ZSTD (L2+) | Fast compression for recent data, high ratio for cold data |
| `enable_blob_files` | false (demo) | Not needed for small metadata values; enable if extended attributes are large |
| `wal_recovery_mode` | `TolerateCorruptedTailRecords` | Tolerate incomplete WAL tail after crash (see §9) |

**Per-CF overrides:**

| CF | `bloom_filter` | `prefix_extractor` | `block_size` |
|---|---|---|---|
| `inodes` | 10 bits/key | None (point lookups only) | 4 KiB |
| `dir_entries` | 10 bits/key | `FixedPrefix(8)` | 4 KiB |
| `system` | None | None | 4 KiB |
| `delta_entries` | 10 bits/key | `FixedPrefix(9)` | 4 KiB |

### 5.5 Delta Entries & Append-Only Write Path

#### 5.5.1 Motivation

Traditional read-modify-write for parent inode attributes (nlink, mtime, ctime) on every `create`/`mkdir`/`unlink`/`rmdir`/`rename` creates a **write amplification bottleneck** on hot directories. A single `mkdir` requires reading the parent inode, deserializing, modifying nlink/mtime/ctime, re-serializing, and writing back — all while holding a lock.

The **delta entries** mechanism (inspired by the Mantle paper's approach to metadata journaling) replaces this pattern with an **append-only** write path: mutations are recorded as small, fixed-size delta operations and folded on read or during background compaction.

#### 5.5.2 Architecture Overview

```
  Write Path (create / mkdir / unlink / rmdir / rename):

  ┌─────────────┐     append      ┌──────────────────┐
  │ MetadataServer │ ────────────► │  DeltaStore       │
  │ append_parent  │               │  (delta_entries CF)│
  │ _deltas()      │               └──────────────────┘
  └───────┬───────┘
          │  apply_deltas()
          ▼
  ┌──────────────────┐
  │ InodeFoldedCache  │  (LRU, in-memory)
  │ keeps hot inodes  │
  │ up-to-date        │
  └──────────────────┘
          │  mark_dirty()
          ▼
  ┌──────────────────────────┐
  │ DeltaCompactionWorker    │  (background thread)
  │ periodically folds dirty │
  │ inodes back to base      │
  └──────────────────────────┘


  Read Path (getattr / lookup):

  ┌─────────────┐   cache hit?   ┌──────────────────┐
  │ load_inode() │ ─────────────► │ InodeFoldedCache  │ → return cached value
  └───────┬──────┘   miss         └──────────────────┘
          │
          ├── read base from MetadataStore (inodes CF)
          ├── scan deltas from DeltaStore (delta_entries CF)
          ├── fold_deltas(base, deltas)
          └── populate cache, return folded value
```

#### 5.5.3 DeltaOp Enum (`server/src/delta.rs`)

A `DeltaOp` represents an incremental modification to an inode's attributes. Instead of modifying the base inode directly, callers append deltas and the system folds them lazily.

```rust
pub enum DeltaOp {
    /// Increment (or decrement) `nlink` by the given signed amount.
    IncrementNlink(i32),
    /// Set `mtime` to the given timestamp (fold takes max).
    SetMtime(u64),
    /// Set `ctime` to the given timestamp (fold takes max).
    SetCtime(u64),
    /// Set `atime` to the given timestamp (fold takes max).
    SetAtime(u64),
}
```

**Binary encoding:** Each `DeltaOp` serializes to a compact binary blob:

| Variant | Format | Size |
|---|---|---|
| `IncrementNlink(i32)` | `[0x01][i32 BE]` | 5 bytes |
| `SetMtime(u64)` | `[0x02][u64 BE]` | 9 bytes |
| `SetCtime(u64)` | `[0x03][u64 BE]` | 9 bytes |
| `SetAtime(u64)` | `[0x04][u64 BE]` | 9 bytes |

#### 5.5.4 Fold Semantics

The `fold_deltas(base, deltas)` function applies a sequence of deltas to a base `InodeValue` **in place**:

- `IncrementNlink(n)` → `base.nlink = max(0, base.nlink as i64 + n as i64) as u32` (clamped to avoid underflow)
- `SetMtime(t)` → `base.mtime = max(base.mtime, t)` (monotonic — latest timestamp wins)
- `SetCtime(t)` → `base.ctime = max(base.ctime, t)`
- `SetAtime(t)` → `base.atime = max(base.atime, t)`

**Key property:** Fold is **commutative for timestamps** (max is order-independent) and **associative for nlink** (integer addition). This means concurrent appends produce correct results regardless of ordering, and partial folds can be resumed.

#### 5.5.5 DeltaStore Trait (`storage/src/lib.rs`)

The `DeltaStore` trait abstracts the delta persistence layer at the raw-byte level:

```rust
pub trait DeltaStore: Send + Sync {
    /// Atomically append one or more serialized delta values for an inode.
    /// Returns the sequence numbers assigned to each delta.
    fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>>;

    /// Scan all pending (un-compacted) deltas for an inode,
    /// returning them in sequence-number order as raw bytes.
    fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;

    /// Scan all pending delta **keys** for an inode, returning the raw key bytes.
    /// Used by the compaction worker to build atomic delete batches
    /// (DeleteDelta operations in a WriteBatch/Transaction).
    fn scan_delta_keys(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;

    /// Delete all deltas for an inode (called after compaction).
    fn clear_deltas(&self, inode: Inode) -> FsResult<()>;
}
```

**Implementations:**

| Implementation | Storage | Use Case |
|---|---|---|
| `MemoryDeltaStore` | In-memory `BTreeMap<(Inode, u64), Vec<u8>>` | Unit tests, demo mode |
| `RocksDeltaStore` | RocksDB `delta_entries` CF | Production, persistent storage |

Each implementation maintains a per-inode monotonic sequence counter (`AtomicU64`) to ensure deltas are ordered within each inode.

#### 5.5.6 InodeFoldedCache (`server/src/cache.rs`)

An LRU cache that stores the **folded** (base + all deltas applied) `InodeValue` for recently accessed inodes.

```rust
pub struct InodeFoldedCache {
    inner: Mutex<CacheInner>,  // thread-safe
}
```

**Operations:**

| Method | Description |
|---|---|
| `get(inode)` | Lookup + promote to MRU. Returns `None` on miss. |
| `put(inode, value)` | Insert/overwrite. Evicts LRU entry if at capacity. |
| `apply_delta(inode, delta)` | Update cached value in-place (no-op on miss). |
| `apply_deltas(inode, deltas)` | Batch in-place update (no-op on miss). |
| `invalidate(inode)` | Remove entry (called after compaction). |

**Why write-through on the write path?** When `append_parent_deltas` is called, the delta is persisted to the `DeltaStore` **and** applied to the cache in the same call. This means subsequent `getattr` calls hit the cache directly without scanning deltas, keeping the hot path at O(1).

#### 5.5.7 Write Path: `append_parent_deltas`

The core helper that replaces read-modify-write on the parent inode:

```rust
fn append_parent_deltas(&self, parent: Inode, deltas: &[DeltaOp]) -> FsResult<()> {
    // 1. Serialize and persist to DeltaStore.
    let serialized: Vec<Vec<u8>> = deltas.iter().map(|d| d.serialize()).collect();
    self.delta_store.append_deltas(parent, &serialized)?;

    // 2. Update cache in-place (write-through).
    self.cache.apply_deltas(parent, deltas);

    // 3. Mark inode as dirty for background compaction.
    self.compaction.mark_dirty(parent);

    Ok(())
}
```

> **Important implementation detail:** In the current code, `append_parent_deltas` is called **outside** the main PCC transaction (after `batch.commit()`). This is intentional:
> - Including delta appends in the transaction would require locking the parent inode row, creating contention on hot directories.
> - If the process crashes after `batch.commit()` but before the delta append, only the parent's `mtime`/`ctime` (and possibly `nlink`) will be stale — the child inode and directory entry are already committed correctly.
> - This trade-off prioritizes write throughput over strict parent-timestamp consistency.

**Operations that use this path:**

| Operation | Deltas Appended to Parent |
|---|---|
| `create` | `SetMtime(now)`, `SetCtime(now)` |
| `mkdir` | `IncrementNlink(1)`, `SetMtime(now)`, `SetCtime(now)` |
| `unlink` | `SetMtime(now)`, `SetCtime(now)` |
| `rmdir` | `IncrementNlink(-1)`, `SetMtime(now)`, `SetCtime(now)` |
| `rename` | `SetMtime(now)`, `SetCtime(now)` (per affected parent) |

#### 5.5.8 Read Path: `load_inode`

The unified read path with cache-first, delta-fold-on-miss strategy:

```rust
fn load_inode(&self, inode: Inode) -> FsResult<InodeValue> {
    // 1. Cache hit → return immediately.
    if let Some(cached) = self.cache.get(inode) {
        return Ok(cached);
    }

    // 2. Read base from MetadataStore.
    let key = encode_inode_key(inode);
    let mut iv = match self.metadata.get(&key)? {
        Some(bytes) => InodeValue::deserialize(&bytes)?,
        None => return Err(FsError::NotFound),
    };

    // 3. Fold pending deltas.
    let raw_deltas = self.delta_store.scan_deltas(inode)?;
    if !raw_deltas.is_empty() {
        let ops: Vec<DeltaOp> = raw_deltas
            .iter()
            .filter_map(|bytes| DeltaOp::deserialize(bytes).ok())
            .collect();
        fold_deltas(&mut iv, &ops);
    }

    // 4. Populate cache for subsequent reads.
    self.cache.put(inode, iv.clone());

    Ok(iv)
}
```

**Performance characteristics:**

| Scenario | Cost |
|---|---|
| Cache hit | O(1) — single `HashMap` lookup + LRU promotion |
| Cache miss, no deltas | 1 MetadataStore read + 1 DeltaStore prefix scan (empty) |
| Cache miss, N deltas | 1 MetadataStore read + 1 DeltaStore prefix scan + O(N) fold |
| After compaction | Cache invalidated → next read loads fresh base (0 deltas) |

#### 5.5.9 Background Compaction Worker (`server/src/compaction.rs`)

The `DeltaCompactionWorker` runs in a background thread and periodically merges accumulated deltas back into the base inode value, keeping read amplification bounded.

```rust
pub struct DeltaCompactionWorker<M, DS>
where
    M: MetadataStore,
    DS: DeltaStore,
{
    metadata: Arc<M>,
    delta_store: Arc<DS>,
    cache: Arc<InodeFoldedCache>,
    config: CompactionConfig,
    dirty: Mutex<HashSet<Inode>>,   // inodes with pending deltas
    running: AtomicBool,            // stop flag
}
```

**Configuration:**

| Parameter | Default | Description |
|---|---|---|
| `interval_ms` | 5,000 ms | How often the worker scans for dirty inodes |
| `delta_threshold` | 32 | Minimum number of pending deltas before compaction triggers |

**Compaction algorithm for a single inode:**

```
  compact_inode(inode):
    1. scan_deltas(inode) → raw_deltas[]
    2. if len(raw_deltas) < threshold → skip (re-mark as dirty)
    3. read base from MetadataStore
    4. fold_deltas(base, raw_deltas)
    5. write updated base to MetadataStore
    6. clear_deltas(inode) in DeltaStore
    7. invalidate(inode) in cache
```

**Lifecycle:**

| Method | Description |
|---|---|
| `mark_dirty(inode)` | Called by write path after appending deltas |
| `compact_dirty()` | One round: swap out dirty set, compact eligible inodes |
| `flush_all()` | Force-compact all dirty inodes regardless of threshold (shutdown / tests) |
| `run_loop()` | Blocking loop: `sleep(interval)` → `compact_dirty()` → repeat until `stop()` |
| `stop()` | Set `running = false`; loop exits after current sleep + final `flush_all()` |

**Crash safety:** If the process crashes between steps 5 and 6 (base written but deltas not cleared), the next `load_inode` will re-fold the same deltas on top of the already-updated base. Because `SetMtime`/`SetCtime` use `max()` semantics and `IncrementNlink` will double-count, the compaction worker should ideally use a WriteBatch to atomically update the base and delete deltas. In the current demo implementation, this is acceptable because:
- Timestamps use `max()` → re-applying is harmless.
- Nlink double-count is rare (only on crash) and detectable via `fsck`-style consistency check.

**Future improvement:** Wrap steps 5–6 in a single `WriteBatch` across `inodes` and `delta_entries` CFs (requires both CFs in the same RocksDB instance).

---

## 6. POSIX Operations — Detailed Design

Each operation follows a uniform template: **interface signature** → **description** → **preconditions** → **step-by-step implementation** → **CF access pattern** → **error mapping**.

### Common Error Mapping

All operations map `FsError` variants to POSIX errno values:

| `FsError` Variant | POSIX errno | Typical Trigger |
|---|---|---|
| `NotFound` | `ENOENT` | Inode or directory entry does not exist |
| `AlreadyExists` | `EEXIST` | Name already exists in directory |
| `IsADirectory` | `EISDIR` | Attempted file operation on a directory |
| `NotADirectory` | `ENOTDIR` | Attempted directory operation on a file |
| `DirectoryNotEmpty` | `ENOTEMPTY` | `rmdir` on non-empty directory |
| `PermissionDenied` | `EACCES` | Permission check failed |
| `InvalidInput(msg)` | `EINVAL` | Invalid mode, name, offset, etc. |
| `TransactionConflict` | `EAGAIN` | PCC transaction conflict after max retries |
| `Io(msg)` | `EIO` | RocksDB or disk I/O failure |
| `NotImplemented` | `ENOSYS` | Operation not yet implemented |
| `Other(msg)` | `EIO` | Catch-all |

Additional errno values are returned by specific operations (documented per-operation below).

---

### 6.1 Metadata Operations

#### 6.1.1 `lookup`

```rust
fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr>;
```

**Description:** Resolve a child entry by name within a parent directory. This is the most frequently called operation — every path component triggers one `lookup`.

**Preconditions:**
- `parent` must be a valid directory inode.
- `name` must be non-empty and ≤ 255 bytes.

**Implementation Steps:**

```rust
fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
    // Step 1: Resolve name → child inode via DirectoryIndex
    let child_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;  // ENOENT if not found

    // Step 2: Load child inode via delta-aware read path (see §5.5.8)
    //   load_inode checks the InodeFoldedCache first, then falls back to
    //   MetadataStore base + DeltaStore fold.
    let iv = self.load_inode(child_inode)?;

    Ok(iv.to_attr())
}
```

**CF Access:** `dir_entries` (read) → `inodes` (read, on cache miss) → `delta_entries` (prefix scan, on cache miss). On cache hit the cost is a single in-memory `HashMap` lookup.

**Error Mapping:**
| Condition | Error |
|---|---|
| `name` not found in parent | `NotFound` → `ENOENT` |
| `parent` inode does not exist or is not a directory | `NotFound` → `ENOENT` |

---

#### 6.1.2 `getattr`

```rust
fn getattr(&self, inode: Inode) -> FsResult<FileAttr>;
```

**Description:** Retrieve the attributes (metadata) of an inode by its ID. This is one of the most frequently called operations — called by `stat()`, `ls -l`, and internally by many other operations.

**Preconditions:**
- `inode` must be a valid allocated inode.

**Implementation Steps:**

```rust
fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
    // Delegates to the delta-aware load_inode (see §5.5.8):
    //   1. Check InodeFoldedCache → return on hit
    //   2. Read base from MetadataStore (inodes CF)
    //   3. Scan pending deltas from DeltaStore (delta_entries CF)
    //   4. Fold deltas into base → populate cache → return
    let iv = self.load_inode(inode)?;
    Ok(iv.to_attr())
}
```

**CF Access:** On cache hit: none (in-memory). On cache miss: `inodes` (read) → `delta_entries` (prefix scan).

**Performance:** With the `InodeFoldedCache` (default capacity 4096), hot inodes are served entirely from memory. Background compaction (§5.5.9) keeps the delta chain short, so cache-miss fold cost stays bounded.

**Error Mapping:**
| Condition | Error |
|---|---|
| Inode not found | `NotFound` → `ENOENT` |

---

#### 6.1.3 `setattr`

```rust
fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr>;
```

Where `SetAttrRequest` uses `Option<T>` to avoid the ambiguity of using 0 to mean "no change":

```rust
pub struct SetAttrRequest {
    pub mode:  Option<u32>,
    pub uid:   Option<u32>,
    pub gid:   Option<u32>,
    pub size:  Option<u64>,
    pub atime: Option<u64>,
    pub mtime: Option<u64>,
}
```

**Description:** Modify attributes of an existing inode. Used for `chmod`, `chown`, `utimes`, `truncate` (size change), etc. Each `Some(value)` field is applied; `None` fields are left unchanged.

**Preconditions:**
- `inode` must exist.
- Caller must have appropriate permissions (owner or root).

**Implementation Steps:**

```rust
fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr> {
    let truncate_size = req.size;

    let (attr, needs_truncate) = self.execute_with_retry(|| {
        let mut batch = self.begin_write();
        let key = encode_inode_key(inode);

        // Step 1: Read current attributes inside PCC transaction (GetForUpdate)
        let raw = batch.get_for_update_inode(&key)?
            .ok_or(FsError::NotFound)?;
        let mut iv = InodeValue::deserialize(&raw)?;
        let ts = now_secs();

        // Step 2: Apply changes selectively (Option<T> — no ambiguity)
        if let Some(mode) = req.mode {
            iv.mode = (iv.mode & 0o170000) | (mode & 0o7777);
        }
        if let Some(uid)   = req.uid   { iv.uid   = uid; }
        if let Some(gid)   = req.gid   { iv.gid   = gid; }
        if let Some(atime) = req.atime { iv.atime = atime; }
        if let Some(mtime) = req.mtime { iv.mtime = mtime; }

        // Step 3: Handle size change (truncate)
        let mut do_truncate = false;
        if let Some(new_size) = truncate_size {
            if new_size != iv.size {
                iv.size = new_size;
                do_truncate = true;
            }
        }

        // Step 4: Update ctime (always changes on setattr)
        iv.ctime = ts;

        // Step 5: Write back inside transaction
        Self::batch_put_inode(batch.as_mut(), inode, &iv);
        batch.commit()?;
        self.cache.put(inode, iv.clone());
        Ok((iv.to_attr(), do_truncate))
    })?;

    // Perform the actual truncate after transaction commit.
    if needs_truncate {
        if let Some(new_size) = truncate_size {
            block_on(self.data_client.truncate(inode, new_size))?;
        }
    }
    Ok(attr)
}
```

**CF Access:** `inodes` (read + write within PCC transaction).

**TOCTOU mitigation:** The read-modify-write is protected by `get_for_update_inode`, which acquires a pessimistic row lock. Concurrent `setattr` calls on the same inode are serialized by the transaction engine; on conflict, `execute_with_retry` retries up to 3 times.

**Error Mapping:**
| Condition | Error |
|---|---|
| Inode not found | `NotFound` → `ENOENT` |
| Caller is not owner and not root | `PermissionDenied` → `EACCES` |
| Transaction conflict after retries | `TransactionConflict` → `EAGAIN` |

---

#### 6.1.4 `statfs`

```rust
fn statfs(&self, inode: Inode) -> FsResult<StatFs>;
```

**Description:** Return filesystem-wide statistics. The `inode` parameter is ignored (FUSE always passes the mount root inode).

**Implementation Steps:**

```rust
fn statfs(&self, _inode: Inode) -> FsResult<StatFs> {
    // Option A: Return pre-configured static values (demo)
    // Option B: Compute from system CF counters + data.img size

    let total_blocks = self.data_total_size / BLOCK_SIZE;       // e.g. data.img size / 4K
    let used_inodes  = self.allocator.next.load(Relaxed) - 2;   // subtract reserved
    let max_inodes   = u64::MAX;                                 // practically unlimited

    Ok(StatFs {
        blocks:  total_blocks,
        bfree:   total_blocks / 2,  // approximate — real impl tracks used blocks
        bavail:  total_blocks / 2,
        files:   max_inodes,
        ffree:   max_inodes - used_inodes,
        bsize:   BLOCK_SIZE as u32, // 4096
        namelen: 255,
    })
}
```

**CF Access:** `system` (read, optional). Mostly computed from in-memory state.

---

### 6.2 Directory Operations

#### 6.2.1 `readdir`

```rust
fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
```

**Description:** List all entries in a directory. Returns `.`, `..`, and all children.

**Implementation Steps:**

```rust
fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
    // Step 1: Verify inode is a directory
    let attr = self.getattr(inode)?;
    if attr.mode & S_IFDIR == 0 {
        return Err(FsError::InvalidInput("not a directory".into())); // ENOTDIR
    }

    // Step 2: Delegate to DirectoryIndex
    self.index.list_dir(inode)
    // list_dir already prepends "." and ".." entries (see §5.1.2)
}
```

**CF Access:** `inodes` (read, verify type) → `dir_entries` (prefix scan).

---

#### 6.2.2 `create`

```rust
fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
```

**Description:** Create a new regular file in the given parent directory.

**Preconditions:**
- `parent` must be a valid directory.
- `name` must not already exist in `parent`.
- Caller must have write + execute permission on `parent`.

**Implementation Steps:**

```rust
fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
    let _guard = self.lock_dir(parent);

    // Step 1: Check name does not already exist
    if self.index.resolve_path(parent, name)?.is_some() {
        return Err(FsError::AlreadyExists);  // EEXIST
    }

    // Step 2: Allocate new inode
    let new_inode = self.allocator.alloc();
    let ts = now_secs();

    // Step 3: Construct InodeValue for new file
    let iv = InodeValue {
        version: 1,
        inode: new_inode,
        size: 0,
        mode: S_IFREG | (mode & 0o7777),
        nlink: 1,
        uid: 0, gid: 0,
        atime: ts, mtime: ts, ctime: ts,
    };

    // Step 4: Persist new inode + dir entry
    self.save_inode(new_inode, &iv)?;          // inodes CF
    self.index.insert_child(parent, name, new_inode, iv.to_attr())?;  // dir_entries CF

    // Step 5: Update parent times via **delta append** (no read-modify-write)
    self.append_parent_deltas(
        parent,
        &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
    )?;

    Ok(iv.to_attr())
}
```

**CF Access:** `dir_entries` (read check + write) → `inodes` (write new child) → `delta_entries` (append parent deltas).

**Delta advantage:** The parent inode is **not read** during `create`. Instead, `SetMtime` and `SetCtime` deltas are appended to the delta store, and the in-memory folded cache is updated. This eliminates the read-modify-write contention on the parent under high concurrency.

**Error Mapping:**
| Condition | Error |
|---|---|
| Parent not found | `NotFound` → `ENOENT` |
| Name already exists | `Other("file exists")` → `EEXIST` |
| No write permission on parent | `PermissionDenied` → `EACCES` |

---

#### 6.2.3 `mkdir`

```rust
fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
```

**Description:** Create a new directory. Similar to `create` but with directory-specific semantics.

**Key Differences from `create`:**

1. **File type:** `S_IFDIR` instead of `S_IFREG`.
2. **nlink initialization:** New directory starts with `nlink=2` (for `.` self-reference and the entry in parent).
3. **Parent nlink update:** Parent directory's `nlink` is incremented by 1 (for `..` reference from child).

**Implementation Steps:**

```rust
fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
    let _guard = self.lock_dir(parent);

    // Step 1: Check name does not already exist
    if self.index.resolve_path(parent, name)?.is_some() {
        return Err(FsError::AlreadyExists);  // EEXIST
    }

    // Step 2: Allocate new inode
    let new_inode = self.allocator.alloc();
    let ts = now_secs();

    // Step 3: Construct InodeValue for new directory
    let iv = InodeValue {
        version: 1,
        inode: new_inode,
        size: 0,
        mode: S_IFDIR | (mode & 0o7777),
        nlink: 2,  // "." and parent entry
        uid: 0, gid: 0,
        atime: ts, mtime: ts, ctime: ts,
    };

    // Step 4: Persist new inode + dir entry
    self.save_inode(new_inode, &iv)?;          // inodes CF
    self.index.insert_child(parent, name, new_inode, iv.to_attr())?;  // dir_entries CF

    // Step 5: Parent nlink +1 (for "..") + update times via **delta append**
    self.append_parent_deltas(
        parent,
        &[
            DeltaOp::IncrementNlink(1),
            DeltaOp::SetMtime(ts),
            DeltaOp::SetCtime(ts),
        ],
    )?;

    Ok(iv.to_attr())
}
```

**CF Access:** `dir_entries` (read check + write) → `inodes` (write new child) → `delta_entries` (append parent deltas: nlink+1, mtime, ctime).

**Delta advantage:** The parent inode's nlink increment is expressed as `IncrementNlink(1)` delta instead of a read-modify-write. Under a burst of concurrent `mkdir` calls, each thread appends its own delta independently — no contention on the parent's base inode value.

---

#### 6.2.4 `unlink`

```rust
fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
```

**Description:** Remove a directory entry for a regular file. If no more references exist (nlink reaches 0), the inode and its data content are eligible for deletion.

**Implementation Steps:**

```rust
fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
    let _guard = self.lock_dir(parent);

    // Step 1: Resolve the target inode
    let child_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;

    // Step 2: Verify target is NOT a directory (use rmdir for directories)
    let mut child_iv = self.load_inode(child_inode)?;
    if Self::is_dir(child_iv.mode) {
        return Err(FsError::IsADirectory);  // EISDIR
    }

    // Step 3: Remove dir entry
    self.index.remove_child(parent, name)?;

    // Step 4: Decrement nlink on child
    child_iv.nlink = child_iv.nlink.saturating_sub(1);

    if child_iv.nlink == 0 {
        // No more references — delete inode
        self.delete_inode(child_inode)?;
    } else {
        // Still has references — update ctime and save
        let ts = now_secs();
        child_iv.ctime = ts;
        self.save_inode(child_inode, &child_iv)?;
    }

    // Step 5: Update parent times via **delta append** (no read-modify-write)
    let ts = now_secs();
    self.append_parent_deltas(
        parent,
        &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
    )?;

    Ok(())
}
```

**Deferred deletion (open-file case):** If the file is currently open (tracked by an in-memory `HashMap<Inode, u32>` of open handle counts), the actual inode + data deletion is deferred until the last `flush`/`close`. The dir entry is removed immediately (POSIX: unlinked files remain accessible via open file descriptors).

**CF Access:** `dir_entries` (read + delete) → `inodes` (read child + write/delete child) → `delta_entries` (append parent deltas: mtime, ctime).

**Delta advantage:** The parent inode update is a pure delta append — no read of the parent's base value is needed.

---

#### 6.2.5 `rmdir`

```rust
fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
```

**Description:** Remove an empty directory.

**Implementation Steps:**

```rust
fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
    let _guard = self.lock_dir(parent);

    // Step 1: Resolve the target
    let child_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;

    // Step 2: Verify target IS a directory
    let child_iv = self.load_inode(child_inode)?;
    if !Self::is_dir(child_iv.mode) {
        return Err(FsError::NotADirectory);  // ENOTDIR
    }

    // Step 3: Check directory is empty
    let entries = self.index.list_dir(child_inode)?;
    if !entries.is_empty() {
        return Err(FsError::DirectoryNotEmpty);  // ENOTEMPTY
    }

    // Step 4: Remove dir entry and delete child inode
    self.index.remove_child(parent, name)?;
    self.delete_inode(child_inode)?;

    // Step 5: Parent nlink -1 + update times via **delta append**
    let ts = now_secs();
    self.append_parent_deltas(
        parent,
        &[
            DeltaOp::IncrementNlink(-1),
            DeltaOp::SetMtime(ts),
            DeltaOp::SetCtime(ts),
        ],
    )?;

    Ok(())
}
```

**CF Access:** `dir_entries` (read + delete) → `inodes` (read child + delete child) → `delta_entries` (append parent deltas: nlink-1, mtime, ctime).

**Delta advantage:** The parent's nlink decrement is expressed as `IncrementNlink(-1)` delta — no read of the parent base value is needed.

**Error Mapping:**
| Condition | Error |
|---|---|
| Target not found | `NotFound` → `ENOENT` |
| Target is not a directory | `InvalidInput` → `ENOTDIR` |
| Directory is not empty | `Other` → `ENOTEMPTY` |

---

#### 6.2.6 `rename`

```rust
fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()>;
```

**Description:** Atomically move/rename a directory entry. This is one of the most complex POSIX operations. It must handle:
- Same-directory rename
- Cross-directory move
- Target already exists (atomic replacement)
- Directory-specific nlink updates

**Implementation Steps:**

```rust
fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()> {
    // Step 1: Acquire locks in inode order to prevent deadlock
    let (_guard1, _guard2) = if parent == new_parent {
        (self.lock_dir(parent), None)
    } else {
        let (first, second) = if parent < new_parent {
            (parent, new_parent)
        } else {
            (new_parent, parent)
        };
        (self.lock_dir(first), Some(self.lock_dir(second)))
    };

    // Step 2: Resolve source
    let src_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;
    let src_iv = self.load_inode(src_inode)?;
    let src_is_dir = Self::is_dir(src_iv.mode);
    let ts = now_secs();

    // Step 3: Handle existing target (POSIX atomic replacement)
    if let Some(dst_inode) = self.index.resolve_path(new_parent, new_name)? {
        let dst_iv = self.load_inode(dst_inode)?;
        let dst_is_dir = Self::is_dir(dst_iv.mode);

        // POSIX constraint: cannot replace dir with non-dir and vice versa
        if src_is_dir && !dst_is_dir {
            return Err(FsError::NotADirectory);  // ENOTDIR
        }
        if !src_is_dir && dst_is_dir {
            return Err(FsError::IsADirectory);   // EISDIR
        }

        // If target is a directory, it must be empty
        if dst_is_dir {
            let entries = self.index.list_dir(dst_inode)?;
            if !entries.is_empty() {
                return Err(FsError::DirectoryNotEmpty);  // ENOTEMPTY
            }
            self.delete_inode(dst_inode)?;
            // Adjust new_parent nlink via delta
            self.append_parent_deltas(new_parent, &[
                DeltaOp::IncrementNlink(-1),
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ])?;
        } else {
            self.delete_inode(dst_inode)?;
        }

        self.index.remove_child(new_parent, new_name)?;
    }

    // Step 4: Move the entry
    self.index.remove_child(parent, name)?;
    self.index.insert_child(new_parent, new_name, src_inode, src_iv.to_attr())?;

    // Step 5: Handle nlink and time updates via **delta append**
    if src_is_dir && parent != new_parent {
        // Cross-directory dir rename:
        // Old parent loses ".." reference → nlink -1
        self.append_parent_deltas(parent, &[
            DeltaOp::IncrementNlink(-1),
            DeltaOp::SetMtime(ts),
            DeltaOp::SetCtime(ts),
        ])?;
        // New parent gains ".." reference → nlink +1
        self.append_parent_deltas(new_parent, &[
            DeltaOp::IncrementNlink(1),
            DeltaOp::SetMtime(ts),
            DeltaOp::SetCtime(ts),
        ])?;
    } else {
        // Same parent or non-dir cross-parent: just update times
        self.append_parent_deltas(parent, &[
            DeltaOp::SetMtime(ts),
            DeltaOp::SetCtime(ts),
        ])?;
        if parent != new_parent {
            self.append_parent_deltas(new_parent, &[
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ])?;
        }
    }

    // Step 6: Update source inode ctime
    let mut src_iv = self.load_inode(src_inode)?;
    src_iv.ctime = ts;
    self.save_inode(src_inode, &src_iv)?;

    Ok(())
}
```

**CF Access:** `dir_entries` (read src + read dst + delete src + put dst) → `inodes` (read/write source + delete target) → `delta_entries` (append parent deltas for nlink/time updates).

**Delta Operations Summary (worst case — cross-dir rename replacing existing dir):**

| Operation | Store | Target |
|---|---|---|
| Delete old dir entry | `DirectoryIndex` | `(parent, name)` |
| Insert new dir entry | `DirectoryIndex` | `(new_parent, new_name)` |
| Delete replaced target inode | `MetadataStore` | `target_inode` |
| Update source inode (ctime) | `MetadataStore` | `src_inode` |
| `IncrementNlink(-1)` delta | `DeltaStore` | `new_parent` (target was dir) |
| `IncrementNlink(-1)` delta | `DeltaStore` | `parent` (lost `..` ref) |
| `IncrementNlink(+1)` delta | `DeltaStore` | `new_parent` (gained `..` ref) |
| `SetMtime` + `SetCtime` deltas | `DeltaStore` | both parents |

**Delta advantage:** All parent nlink/time updates are expressed as delta appends. No parent base value is read during rename, eliminating read-modify-write contention on hot directories.

---

### 6.3 Data Operations

> **Architecture note:** In the current implementation, data read/write operations are **not handled by `MetadataServer`**. Instead, the `VfsCore` routing layer (in the client crate) coordinates between `MetadataOps` and `DataOps`:
>
> 1. `open()` → `MetadataOps::open()` → returns `OpenResponse { handle, data_location }`.
> 2. `read()` / `write()` → `DataOps::read_data()` / `DataOps::write_data()` directly (bypassing MetadataServer).
> 3. After a successful `write()`, `VfsCore` calls `MetadataOps::report_write(inode, new_size, mtime)` to update file size and mtime.

#### 6.3.1 `open`

```rust
fn open(&self, inode: Inode, flags: u32) -> FsResult<OpenResponse>;
```

**Description:** Open a file and return a file handle plus `DataLocation` (DataServer endpoint info). The client uses `DataLocation` to know which DataServer to contact for subsequent read/write.

**Implementation Steps:**

```rust
fn open(&self, inode: Inode, _flags: u32) -> FsResult<OpenResponse> {
    // Step 1: Verify inode exists and is a regular file
    let iv = self.load_inode(inode)?;
    if Self::is_dir(iv.mode) {
        return Err(FsError::IsADirectory);
    }

    // Step 2: Return handle + DataLocation
    Ok(OpenResponse {
        handle: 0,  // We don't track open files yet (see TODO: deferred unlink).
        data_location: self.data_location.clone(),
    })
}
```

**CF Access:** `inodes` (read). No writes.

**Note:** Permission checks based on `flags` (O_RDONLY/O_WRONLY/O_RDWR) are not yet implemented (see TODO T-21).

---

#### 6.3.2 `read` (via VfsCore)

```rust
// VfsCore routes read directly to DataServer:
async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
    self.data.read_data(inode, offset, size).await
}
```

**Description:** Read file content at the given offset. The `VfsCore` bypasses `MetadataServer` entirely and reads directly from the `DataServer`.

**CF Access:** None (MetadataServer not involved). DataStore only.

---

#### 6.3.3 `write` (via VfsCore)

```rust
// VfsCore coordinates data write + metadata update:
async fn write(&self, inode: Inode, offset: u64, data: &[u8], _flags: u32) -> FsResult<u32> {
    // Step 1: Write directly to DataServer.
    let written = self.data.write_data(inode, offset, data).await?;

    // Step 2: Report the write back to MetadataServer to update size/mtime.
    let new_end = offset + written as u64;
    let ts = now_secs();
    self.metadata.report_write(inode, new_end, ts).await?;

    Ok(written)
}
```

**`MetadataOps::report_write`:** A method (not in the original design) that allows the client to notify the MetadataServer after a successful data write. The MetadataServer atomically updates `size` (only if `new_end > current_size`) and `mtime`/`ctime` using a PCC transaction.

**Consistency note:** The data write and metadata update are **not** in the same transaction (DataStore is a separate engine). If the process crashes after data write but before `report_write`:
- File content is written but size/mtime is stale.
- On recovery, the metadata still shows old size → reads are bounded to old size → extra data beyond old size is invisible but harmless.

---

#### 6.3.4 `flush`

```rust
fn flush(&self, inode: Inode) -> FsResult<()>;
```

**Description:** Flush buffered data for a file. Called on `close()`. In FUSE, `flush` may be called multiple times if a file is `dup()`-ed.

**Implementation Steps:**

```rust
fn flush(&self, inode: Inode) -> FsResult<()> {
    // Step 1: Flush DataStore buffers to disk
    block_on(self.data.flush(inode))?;

    // Step 2: Decrement open handle count (if tracking)
    //   if self.open_handles[inode] -= 1 == 0 {
    //       // Last handle closed
    //       if inode_nlink == 0 {
    //           // Deferred unlink: delete inode and data now
    //           self.metadata.delete(&encode_inode_key(inode))?;
    //       }
    //   }

    Ok(())
}
```

**CF Access:** DataStore (flush) → optionally `inodes` (delete if deferred unlink).

---

#### 6.3.5 `fsync`

```rust
fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()>;
```

**Description:** Force data (and optionally metadata) to persistent storage.

**Implementation Steps:**

```rust
fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
    // Step 1: Always sync data
    block_on(self.data.flush(inode))?;
    //   RawDiskDataStore.flush() calls file.sync_data()

    // Step 2: If datasync=false, also sync metadata
    if !datasync {
        // Force RocksDB WAL flush for this inode's metadata
        // In practice, RocksDB flushes WAL on every write by default (sync_wal=true)
        // So this is a no-op unless we batch metadata writes
    }

    Ok(())
}
```

**`datasync` parameter:**
- `true` → sync only file data (like `fdatasync()`). Skip metadata flush.
- `false` → sync both data and metadata (like `fsync()`).

**CF Access:** DataStore (flush). RocksDB WAL is already durable by default.

---

### 6.4 VfsCore — Client-Side Routing Layer

> **Not in original design.** The code introduces a `VfsCore` struct in the client crate that routes `VfsOps` requests to the appropriate backend:

```rust
pub struct VfsCore {
    metadata: Arc<dyn MetadataOps>,
    data: Arc<dyn DataOps>,
    handle_cache: Mutex<HashMap<u64, String>>,
}
```

`VfsCore` implements `VfsOps` and provides the single routing logic shared by both `EmbeddedClient` (demo mode) and future `RucksClient` (gRPC mode). Key routing rules:

| VfsOps method | Routed to | Notes |
|---|---|---|
| lookup, getattr, readdir, create, mkdir, unlink, rmdir, rename, setattr, statfs | `MetadataOps` | Transparent delegation |
| open | `MetadataOps::open` → extract `DataLocation` → cache handle | Returns just the handle to FUSE |
| read | `DataOps::read_data` | Bypasses MetadataServer entirely |
| write | `DataOps::write_data` → `MetadataOps::report_write` | Two-step coordination (see §6.3.3) |
| flush, fsync | `DataOps::flush` | Bypasses MetadataServer |

### 6.5 `MetadataOps::report_write` — Post-Write Size/Mtime Update

> **Not in original design.** Added to support the split data path where `VfsCore` writes data directly to `DataServer` and then notifies `MetadataServer`.

```rust
async fn report_write(&self, inode: Inode, new_size: u64, mtime: u64) -> FsResult<()>;
```

Uses PCC transaction (`execute_with_retry`) to atomically read the inode, update `size` (only if `new_size > current_size`), set `mtime` and `ctime`, and commit. This ensures that concurrent writes to the same file correctly advance the file size.

### 6.6 `DataOps::delete_data` — Inode Data Cleanup

> **Not in original design.** The `DataOps` trait includes `delete_data(inode)` for cleaning up file content after `unlink` removes the last reference (nlink=0). The `MetadataServer` calls `self.data_client.delete_data(inode)` **outside** the PCC transaction to avoid holding locks during potentially slow I/O.

### 6.7 Inode Allocator Persistence — Outside Transaction

> **Implementation note:** The `InodeAllocator::persist()` call is performed **outside** the PCC transaction (after `batch.commit()` succeeds). This avoids making the allocator counter a contention hot-key inside every create/mkdir transaction. Trade-off: if the process crashes after `commit()` but before `persist()`, the on-disk counter may lag behind. On restart, the counter resumes from the persisted value, and the "phantom" inode IDs (committed in the transaction but not reflected in the counter) are harmlessly skipped — they already have valid metadata in the `inodes` CF.

---

## 7. Transactions & Consistency Guarantees

### 7.1 RocksDB WriteBatch Atomicity

RocksDB `WriteBatch` is the primary mechanism for atomic multi-key mutations. A `WriteBatch` groups multiple put/delete operations across **multiple Column Families** within the same DB instance into a single atomic unit:

- **All-or-nothing:** Either all operations in the batch are applied, or none are (crash during write → WAL replay restores to pre-batch state).
- **Cross-CF:** A single batch can write to `inodes`, `dir_entries`, and `system` CFs simultaneously.
- **No isolation:** WriteBatch does not provide snapshot isolation. Concurrent readers may see intermediate states unless additional locking is used.

### 7.2 Operations Requiring Atomicity

The following operations involve multi-key mutations that **must** be atomic:

| Operation | WriteBatch Contents | CFs Involved |
|---|---|---|
| `create` | Put new inode + Put dir entry + Append parent delta (`IncNlink`/`SetMtime`) + Persist allocator | `inodes`, `dir_entries`, `delta_entries`, `system` |
| `mkdir` | Put new inode + Put dir entry + Append parent delta (`IncNlink`, `SetMtime`) + Persist allocator | `inodes`, `dir_entries`, `delta_entries`, `system` |
| `unlink` | Delete dir entry + Update/Delete child inode (nlink) + Append parent delta (`DecNlink`, `SetMtime`) | `inodes`, `dir_entries`, `delta_entries` |
| `rmdir` | Delete dir entry + Delete child inode + Append parent delta (`DecNlink`, `SetMtime`) | `inodes`, `dir_entries`, `delta_entries` |
| `rename` | Delete old entry + Put new entry + Update/Delete target + Append parent deltas + Update source ctime | `inodes`, `dir_entries`, `delta_entries` |
| `compaction` | Fold deltas into base inode + Clear delta entries for compacted inode | `inodes`, `delta_entries` |

> **Note:** Parent directory inode updates (nlink, mtime) are now recorded as append-only delta entries rather than read-modify-write on the parent inode. This eliminates contention on hot parent directories. The compaction worker periodically folds accumulated deltas back into base inodes.

Operations that are single-key writes (`setattr`, `write` metadata update) do not require WriteBatch but still benefit from RocksDB's WAL durability.

### 7.3 Concurrency Control Strategy

The implementation uses **RocksDB Pessimistic Concurrency Control (PCC) transactions** with an `execute_with_retry` helper to handle transient conflicts.

#### Strategy: PCC TransactionDB + `execute_with_retry`

Every mutating operation follows this template:

```rust
fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T>
where
    F: FnMut() -> FsResult<T>,
{
    for attempt in 0..TXN_MAX_RETRIES {   // TXN_MAX_RETRIES = 3
        match f() {
            Ok(v) => return Ok(v),
            Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                continue;   // retry on transient conflict
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
```

**Within each transaction closure:**

1. `self.begin_write()` creates a RocksDB `Transaction` (PCC mode) via `StorageBundle`.
2. `batch.get_for_update_inode(key)` / `batch.get_for_update_dir_entry(key)` reads the value **and acquires a pessimistic row lock** on that key.
3. All writes are added to the transaction via `batch.push(BatchOp::...)`.
4. `batch.commit()` atomically commits all operations. If another transaction holds a conflicting lock, `commit()` returns `FsError::TransactionConflict`.

**Lock acquisition rules for `rename` (deadlock prevention):**
For cross-directory `rename`, all involved inodes are collected, sorted by inode ID, deduplicated, and then locked in order:

```rust
let mut inode_ids = vec![src_inode];
if let Some(dst_ino) = dst_inode { inode_ids.push(dst_ino); }
if !inode_ids.contains(&parent) { inode_ids.push(parent); }
if parent != new_parent && !inode_ids.contains(&new_parent) {
    inode_ids.push(new_parent);
}
inode_ids.sort_unstable();
inode_ids.dedup();
for &ino in &inode_ids { batch.get_for_update_inode(&encode_inode_key(ino))?; }
```

**Advantages over per-directory mutex (previous design):**
- No `DashMap<Inode, Arc<Mutex<()>>>` needed — eliminates application-level lock management.
- Row-level granularity — two `create` calls in different directories proceed concurrently without any contention.
- Deadlock-free — RocksDB PCC detects lock ordering violations; `execute_with_retry` handles transient conflicts.
- Consistent with `WriteBatch` atomicity — the transaction commit is the WriteBatch commit.

### 7.4 TOCTOU Prevention

**Time-of-Check to Time-of-Use** race conditions occur when a value read during validation is stale by the time it is used. The PCC transaction model (`get_for_update` + `execute_with_retry`) handles most scenarios:

| Scenario | Risk | Mitigation |
|---|---|---|
| `create`: check name doesn't exist, then insert | Another thread creates same name between check and insert | `get_for_update_dir_entry` acquires row lock on the dir key — second transaction blocks or conflicts |
| `setattr`: read attr, modify, write back | Another thread modifies attr between read and write | `get_for_update_inode` acquires row lock — concurrent setattr retries via `execute_with_retry` |
| `rmdir`: check dir is empty, then delete | Another thread creates child between check and delete | Dir entry and child inode are locked; however `list_dir` currently reads outside the transaction (see TODO: rmdir TOCTOU) |
| `rename`: check target exists, then replace | Another thread modifies target between check and replace | All involved dir entries and inodes are locked via `get_for_update` in sorted inode order |

---

## 8. Security Mechanisms

### 8.1 POSIX Permission Model

Every inode stores `mode` (permission bits), `uid` (owner), and `gid` (group). Permission checks follow standard POSIX rules:

```rust
fn check_permission(attr: &FileAttr, required: u32, caller_uid: u32, caller_gid: u32) -> FsResult<()> {
    // Root bypasses all checks
    if caller_uid == 0 { return Ok(()); }

    let mode = attr.mode & 0o777; // lower 9 bits

    let effective = if caller_uid == attr.uid {
        (mode >> 6) & 0o7  // owner bits
    } else if caller_gid == attr.gid {
        (mode >> 3) & 0o7  // group bits
    } else {
        mode & 0o7          // other bits
    };

    if effective & required == required {
        Ok(())
    } else {
        Err(FsError::PermissionDenied)
    }
}
```

**Permission bit meanings:**
| Bit | Value | File | Directory |
|---|---|---|---|
| Read (r) | 4 | Read content | List entries (readdir) |
| Write (w) | 2 | Modify content | Create/delete entries (create, unlink, rename) |
| Execute (x) | 1 | Execute as program | Traverse (lookup, access children) |

**Where checks are performed in each operation:**

| Operation | Check | Permission |
|---|---|---|
| `lookup` | Parent directory | Execute (x) |
| `readdir` | Target directory | Read (r) |
| `create` / `mkdir` | Parent directory | Write + Execute (wx) |
| `unlink` / `rmdir` | Parent directory | Write + Execute (wx) |
| `rename` | Both parent directories | Write + Execute (wx) |
| `open` (O_RDONLY) | Target file | Read (r) |
| `open` (O_WRONLY) | Target file | Write (w) |
| `open` (O_RDWR) | Target file | Read + Write (rw) |
| `setattr` | Target inode | Owner or root |
| `getattr` | *(none)* | No check (POSIX allows stat on any visible inode) |
| `statfs` | *(none)* | No check |

### 8.2 RPC Authentication Integration

The `rpc` crate implements Bearer Token authentication:

```
Client → gRPC Request + Header: "authorization: Bearer <token>"
         ↓
Server → auth.rs: extract token, constant-time compare with configured secret
         ↓
         If mismatch → return gRPC Status::Unauthenticated
         If match → extract caller identity (uid, gid) from token claims or metadata
         ↓
         Pass (uid, gid) to MetadataOps / DataOps methods for permission checks
```

**In demo mode (single binary):** Authentication is bypassed. The caller identity is hardcoded (e.g., uid=1000, gid=1000) or derived from the process's real UID.

**In distributed mode:** The gRPC interceptor (`tonic::service::interceptor`) validates the Bearer Token before any `MetadataOps` / `DataOps` method is invoked. The token is verified using constant-time comparison to prevent timing attacks.

### 8.3 Data Integrity

**Current design (demo):** No explicit integrity checksums on stored data. Reliance on:
- RocksDB internal CRC32 checksums on SSTable blocks (enabled by default).
- Filesystem-level checksums on the raw data file (if using ZFS/Btrfs as host FS).

**Future enhancement:**
- Add a CRC32 field to `InodeValue` that checksums the file content.
- Verify on read, recompute on write.
- For the raw disk data store, append a 4-byte CRC32 trailer per block.

---

## 9. Fault Tolerance & Crash Recovery

### 9.1 Failure Scenarios & Expected Behavior

| # | Failure Scenario | Expected Behavior | Recovery Action |
|---|---|---|---|
| F1 | **Process crash during WriteBatch** | WriteBatch is not yet committed → no partial state on disk. WAL records only complete batches. | RocksDB replays WAL on next open. Incomplete batch entries are discarded. |
| F2 | **Process crash after data write, before metadata update** | File content is written to `data.img` but `inodes` CF still has old `size`/`mtime`. | On recovery, metadata shows old size. Data beyond old size is invisible but harmless. Next write will overwrite or extend correctly. |
| F3 | **RocksDB write failure (disk full, corruption)** | `WriteBatch::write()` returns error. | Return `FsError::Io` to caller. No partial state — WriteBatch is atomic. Caller can retry after freeing disk space. |
| F4 | **Disk I/O error on data.img** | `pwrite`/`pread` returns `errno` (e.g., `EIO`). | Return `FsError::Io` to caller. The specific inode's data may be corrupt, but other inodes are unaffected (no shared state between inode regions). |
| F5 | **Network partition (distributed mode)** | gRPC calls time out. Client receives `Status::Unavailable`. | Client retries with backoff. Server state is not affected — all operations are idempotent or atomic. |
| F6 | **Partial pwrite to data.img** | POSIX does not guarantee `pwrite` atomicity for writes > `PIPE_BUF`. A large write may be partially applied. | On recovery: metadata still has old size → partial write beyond old size is invisible. Partial write within old size → data corruption for that region. Mitigation: write in ≤4KiB chunks (block-aligned). |

### 9.2 RocksDB WAL Crash Consistency

RocksDB's Write-Ahead Log ensures crash consistency for all metadata operations:

```
  Write Operation Flow:

  1. Client calls create(parent, name, mode)
  2. MetadataServer constructs WriteBatch
  3. WriteBatch → WAL (sequential append to log file)
     ├── WAL entry includes ALL operations in the batch
     └── fsync() on WAL file (ensures durability)
  4. WriteBatch → MemTable (in-memory update)
  5. Return success to client

  Crash Recovery Flow:

  1. RocksDB::Open() is called
  2. WAL is scanned from last checkpoint
  3. Complete WAL entries → replayed into MemTable
  4. Incomplete WAL tail → discarded (TolerateCorruptedTailRecords mode)
  5. MemTable reflects consistent state
  6. Normal operation resumes
```

**Key guarantee:** If `db.write(batch)` returns `Ok(())`, all operations in the batch are durable. If the process crashes before `Ok(())` is returned, the batch is either fully recovered from WAL or fully absent.

### 9.3 RawDiskDataStore Recovery

The raw file data store has weaker consistency guarantees than RocksDB:

**Consistency model:** "Last writer wins" with potential partial writes.

**Recovery strategy:**
1. **Inode metadata is the source of truth.** The `size` field in `InodeValue` defines the valid data range for each inode.
2. **Data beyond `size` is garbage.** `read_at` always clamps reads to `attr.size`. Any data written beyond the recorded size (due to crash) is invisible.
3. **Block-aligned writes reduce risk.** By writing in 4KiB-aligned chunks, we align with filesystem block boundaries, minimizing the chance of torn writes.

**Fsync discipline:**
- `flush()` calls `file.sync_data()` which translates to `fdatasync()` — ensures data blocks are on disk.
- Critical metadata updates (size, mtime) are committed to RocksDB **after** data write succeeds.
- If data write succeeds but metadata update fails → stale metadata is safe (see F2 above).
- If data write fails → no metadata update occurs → consistent state.

### 9.4 Inode Allocator Recovery

The inode allocator persists its counter in the `system` CF as part of `WriteBatch` (see §4.2). Recovery scenarios:

| Scenario | Persisted Counter | In-Memory Counter | Effect |
|---|---|---|---|
| Normal shutdown | Matches in-memory | N/A (process exiting) | No issue |
| Crash after alloc(), before WriteBatch commit | Old value (N) | N+1 (lost) | On restart, counter loads N. Inode N+1 was never committed to any CF → no dangling references. |
| Crash after WriteBatch commit | New value (N+1) | N+1 | Consistent. |

**Invariant:** The persisted counter is always ≤ the highest committed inode ID + 1. Phantom allocations (in-memory but not persisted) are harmless because they have no associated metadata or directory entries.

---

## 10. Configuration & Tuning Recommendations

### 10.1 RocksDB Configuration Summary

The following table consolidates all recommended RocksDB parameters for the rucksfs metadata engine. These values are tuned for a metadata-heavy workload with small keys/values and frequent point lookups.

| Category | Parameter | Value | Rationale |
|---|---|---|---|
| **Memory** | `write_buffer_size` | 64 MiB | Adequate for metadata write bursts without excessive memory |
| | `max_write_buffer_number` | 3 | 1 active + 2 flushing = smooth write pipeline |
| | `block_cache` (shared) | 128 MiB | Cache hot inode blocks; increase to 256 MiB+ for large datasets |
| **Compaction** | `level_compaction_dynamic_level_bytes` | true | Auto-size per-level targets; reduces write amplification |
| | `target_file_size_base` | 64 MiB | Matches metadata record density |
| | `max_background_compactions` | 4 | Parallelize compaction on multi-core |
| | `max_background_flushes` | 2 | Ensure MemTable flushes don't stall |
| **Compression** | L0–L1 | LZ4 | Fast; minimal CPU overhead for recent data |
| | L2+ | ZSTD (level 3) | Higher ratio for cold data; good balance |
| **Bloom Filter** | `inodes` CF | 10 bits/key | ~1% FPR; essential for point lookups |
| | `dir_entries` CF | 10 bits/key | Helps prefix-scan skip irrelevant blocks |
| | `system` CF | None | Too few keys to benefit |
| **Prefix** | `dir_entries` `prefix_extractor` | `FixedPrefix(8)` | First 8 bytes = parent inode; enables efficient `list_dir` |
| **WAL** | `wal_recovery_mode` | `TolerateCorruptedTailRecords` | Safe default for crash recovery |
| | `manual_wal_flush` | false | Auto-flush WAL on every write for durability |
| **Block** | `block_size` | 4 KiB | Match OS page size; good for small-value workloads |

### 10.2 RawDiskDataStore Configuration

| Parameter | Default | Description |
|---|---|---|
| `data_file_path` | `./data.img` | Path to the raw disk image file |
| `max_file_size` | 16 MiB | Maximum content size per inode. Determines disk offset formula. |
| `block_size` | 4 KiB | Write alignment unit. Writes are not required to be aligned, but aligned writes reduce torn-write risk. |

**Sizing formula:** Total `data.img` size = `max_inodes * max_file_size`. For example, 10,000 inodes × 16 MiB = 160 GiB. In demo mode, start small (e.g., 1000 inodes × 16 MiB = 16 GiB) and use sparse files (the OS allocates blocks only on first write).

### 10.3 FUSE Mount Options

| Option | Recommended | Effect |
|---|---|---|
| `allow_other` | Yes (if multi-user) | Allow non-root users to access the mount |
| `default_permissions` | Yes | Delegate permission checks to kernel VFS (reduces FUSE roundtrips) |
| `max_read` | 131072 (128 KiB) | Maximum read request size |
| `max_write` | 131072 (128 KiB) | Maximum write request size |
| `noatime` | Yes | Disable atime updates on read (avoids write amplification) |

### 10.4 Performance Tuning Checklist

1. **Enable `noatime`** — Eliminates a metadata write on every read operation.
2. **Increase `block_cache`** — If the working set of inodes fits in cache, lookup/getattr become memory-only operations.
3. **Use `sync_wal = false` for batch operations** — When importing many files, disable per-write WAL sync and call `FlushWAL()` at the end.
4. **Tune `max_file_size`** — Smaller values waste less space but limit file size. Profile actual workloads to find the sweet spot.
5. **Pre-allocate `data.img`** — Use `fallocate()` to pre-allocate the entire file, avoiding fragmentation on the host filesystem.

---

## 11. Glossary

| Term | Definition |
|---|---|
| **Inode** | A unique integer identifier (u64) for a file or directory in the filesystem. Maps to metadata (attributes) and content (data blocks). |
| **CF (Column Family)** | A logical partition within a RocksDB database. Each CF has its own MemTable and SSTable set, but shares the WAL with other CFs in the same DB instance. |
| **WAL (Write-Ahead Log)** | A sequential, append-only log in RocksDB. All writes are recorded in the WAL before being applied to the MemTable, ensuring crash recovery. |
| **WriteBatch** | A RocksDB API for grouping multiple put/delete operations into a single atomic write. Used for multi-key mutations (e.g., create, rename). |
| **MemTable** | An in-memory data structure (typically SkipList) where RocksDB buffers recent writes before flushing to disk as SSTables. |
| **SSTable (Sorted String Table)** | An immutable, sorted, on-disk file produced by flushing a MemTable. SSTables are organized in levels (L0, L1, … LN) and periodically compacted. |
| **LSM-tree (Log-Structured Merge Tree)** | The core data structure of RocksDB. Optimizes write throughput by buffering writes in memory and merging them into sorted on-disk levels through compaction. |
| **Bloom Filter** | A probabilistic data structure used by RocksDB to quickly determine whether a key exists in an SSTable, avoiding unnecessary disk reads. |
| **Compaction** | A background process in RocksDB that merges SSTables across levels, removing deleted/overwritten entries and maintaining sorted order. |
| **nlink (Hard Link Count)** | The number of directory entries pointing to an inode. A regular file starts with nlink=1; a directory starts with nlink=2 (self `.` + parent entry). When nlink reaches 0, the inode is eligible for deletion. |
| **Dentry (Directory Entry)** | A mapping from a (parent\_inode, child\_name) pair to a child inode. Stored in the `dir_entries` CF. |
| **TOCTOU (Time-of-Check to Time-of-Use)** | A race condition where the state checked before an operation changes before the operation executes. Mitigated by holding locks during the read-modify-write cycle. |
| **S_IFDIR / S_IFREG** | POSIX file type constants. `S_IFDIR = 0o040000` (directory), `S_IFREG = 0o100000` (regular file). Stored in the upper bits of the `mode` field. |
| **pread / pwrite** | POSIX system calls for positional I/O. They read/write at a specified offset without modifying the file descriptor's seek position, enabling thread-safe concurrent access. |
| **FUSE (Filesystem in Userspace)** | A Linux kernel interface that allows implementing filesystems as user-space programs. The `fuser` crate provides the Rust binding. |
| **gRPC** | A high-performance RPC framework using Protocol Buffers for serialization and HTTP/2 for transport. Used for client-server communication in rucksfs. |
| **Bearer Token** | An authentication scheme where the client includes a secret token in the HTTP `Authorization` header. The server validates it before processing requests. |
| **bincode** | A compact binary serialization format for Rust. Referenced in the original design but **replaced** by hand-crafted big-endian encoding in the implementation for deterministic byte layout. |
| **fdatasync** | A POSIX system call that flushes file data to disk without flushing metadata (more efficient than `fsync` when only data durability is needed). |
| **Delta Entry** | An append-only record in the `delta_entries` CF that captures a single incremental mutation to an inode's metadata (e.g., nlink change, mtime/ctime update). Multiple delta entries accumulate between compaction cycles and are folded on read to reconstruct the current inode state. |
| **DeltaOp** | An enum representing the possible delta operations: `AdjustNlink(i64)` changes the hard-link count; `SetMtime(SystemTime)` / `SetCtime(SystemTime)` update timestamps; `SetSize(u64)` updates the file size. |
| **Fold (Delta Folding)** | The process of applying a sequence of `DeltaOp` entries, in append order, on top of a base `InodeAttributes` snapshot to produce the up-to-date inode state. Performed on each read (`load_inode`) and during compaction. |
| **Compaction Worker** | A background task (`CompactionWorker`) that periodically scans the `delta_entries` CF, folds accumulated deltas into the base inode in `inode_attributes`, and then clears the processed deltas. This bounds the number of deltas per inode and keeps read latency low. |
| **InodeFoldedCache** | An in-memory LRU cache (`DashMap`-based) that stores recently folded `InodeAttributes`. On cache hit the server skips the RocksDB base-read + delta-scan, significantly reducing read latency for hot inodes. The cache is invalidated on write operations and after compaction. |
| **DeltaStore** | A storage trait that abstracts delta entry persistence. Provides `append_delta`, `scan_deltas`, `scan_delta_keys`, and `clear_deltas` methods, allowing the server to be generic over different storage backends. |
| **StorageBundle** | A trait that owns references to all underlying storage backends and can create an `AtomicWriteBatch` spanning all of them. The `RocksStorageBundle` implementation creates PCC transactions. |
| **AtomicWriteBatch** | A trait representing a collection of `BatchOp` operations committed atomically. Supports `get_for_update_inode` and `get_for_update_dir_entry` for pessimistic row locking. |
| **VfsCore** | A client-side routing layer that implements `VfsOps` by delegating metadata operations to `MetadataOps` and data operations to `DataOps`. Shared by both `EmbeddedClient` and future `RpcClient`. |
| **report_write** | A `MetadataOps` method called by `VfsCore` after a successful data write, updating the inode's `size` and `mtime`/`ctime` in the metadata store. |
| **execute_with_retry** | A helper in `MetadataServer` that retries a transaction closure up to 3 times on `FsError::TransactionConflict`. |
| **TransactionConflict** | An `FsError` variant indicating that a PCC transaction detected a conflicting row lock. Mapped to `EAGAIN` in FUSE responses. |
| **PCC (Pessimistic Concurrency Control)** | A transaction isolation strategy where row locks are acquired at read time (`GetForUpdate`). Prevents conflicts by blocking concurrent access rather than detecting conflicts at commit time (as in OCC). |

**Invariant:** The persisted allocator counter is always ≤ the highest committed inode ID + 1. Phantom allocations (in-memory but not persisted) are harmless because they have no associated metadata or directory entries.

---

## 10. Configuration & Tuning Recommendations

### 10.1 RocksDB Configuration Summary

The following table consolidates all recommended RocksDB parameters for the rucksfs metadata engine. These values are tuned for a metadata-heavy workload with small keys/values and frequent point lookups.

| Category | Parameter | Value | Rationale |
|---|---|---|---|
| **Memory** | `write_buffer_size` | 64 MiB | Adequate for metadata write bursts without excessive memory |
| | `max_write_buffer_number` | 3 | 1 active + 2 flushing = smooth write pipeline |
| | `block_cache` (shared) | 128 MiB | Cache hot inode blocks; increase to 256 MiB+ for large datasets |
| **Compaction** | `level_compaction_dynamic_level_bytes` | true | Auto-size per-level targets; reduces write amplification |
| | `target_file_size_base` | 64 MiB | Matches metadata record density |
| | `max_background_compactions` | 4 | Parallelize compaction on multi-core |
| | `max_background_flushes` | 2 | Ensure MemTable flushes don't stall |
| **Compression** | L0–L1 | LZ4 | Fast; minimal CPU overhead for recent data |
| | L2+ | ZSTD (level 3) | Higher ratio for cold data; good balance |
| **Bloom Filter** | `inodes` CF | 10 bits/key | ~1% FPR; essential for point lookups |
| | `dir_entries` CF | 10 bits/key | Helps prefix-scan skip irrelevant blocks |
| | `system` CF | None | Too few keys to benefit |
| **Prefix** | `dir_entries` `prefix_extractor` | `FixedPrefix(8)` | First 8 bytes = parent inode; enables efficient `list_dir` |
| **WAL** | `wal_recovery_mode` | `TolerateCorruptedTailRecords` | Safe default for crash recovery |
| | `manual_wal_flush` | false | Auto-flush WAL on every write for durability |
| **Block** | `block_size` | 4 KiB | Match OS page size; good for small-value workloads |

### 10.2 RawDiskDataStore Configuration

| Parameter | Default | Description |
|---|---|---|
| `data_file_path` | `./data.img` | Path to the raw disk image file |
| `max_file_size` | 16 MiB | Maximum content size per inode. Determines disk offset formula. |
| `block_size` | 4 KiB | Write alignment unit. Writes are not required to be aligned, but aligned writes reduce torn-write risk. |

**Sizing formula:** Total `data.img` size = `max_inodes * max_file_size`. For example, 10,000 inodes × 16 MiB = 160 GiB. In demo mode, start small (e.g., 1000 inodes × 16 MiB = 16 GiB) and use sparse files (the OS allocates blocks only on first write).

### 10.3 FUSE Mount Options

| Option | Recommended | Effect |
|---|---|---|
| `allow_other` | Yes (if multi-user) | Allow non-root users to access the mount |
| `default_permissions` | Yes | Delegate permission checks to kernel VFS (reduces FUSE roundtrips) |
| `max_read` | 131072 (128 KiB) | Maximum read request size |
| `max_write` | 131072 (128 KiB) | Maximum write request size |
| `noatime` | Yes | Disable atime updates on read (avoids write amplification) |

### 10.4 Performance Tuning Checklist

1. **Enable `noatime`** — Eliminates a metadata write on every read operation.
2. **Increase `block_cache`** — If the working set of inodes fits in cache, lookup/getattr become memory-only operations.
3. **Use `sync_wal = false` for batch operations** — When importing many files, disable per-write WAL sync and call `FlushWAL()` at the end.
4. **Tune `max_file_size`** — Smaller values waste less space but limit file size. Profile actual workloads to find the sweet spot.
5. **Pre-allocate `data.img`** — Use `fallocate()` to pre-allocate the entire file, avoiding fragmentation on the host filesystem.


