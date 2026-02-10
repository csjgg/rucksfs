# rucksfs Technical Design Document

> **Version:** 0.1.0-draft
> **Last Updated:** 2026-02-10
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
  - [8. Security Mechanisms](#8-security-mechanisms)
  - [9. Fault Tolerance \& Crash Recovery](#9-fault-tolerance--crash-recovery)
  - [10. Configuration \& Tuning Recommendations](#10-configuration--tuning-recommendations)
  - [11. Glossary](#11-glossary)

---

## 1. Overview

### 1.1 Project Summary

rucksfs is a user-space file system implemented in Rust. It exposes a standard POSIX interface via Linux FUSE (`fuser` crate), with all storage logic handled by a pluggable server backend. The system is structured as a Cargo workspace containing six crates:

| Crate | Role |
|-------|------|
| `core` | Shared types (`FileAttr`, `DirEntry`, `StatFs`, `FsError`) and trait definitions (`PosixOps`, `ClientOps`) |
| `storage` | Storage trait abstractions (`MetadataStore`, `DataStore`, `DirectoryIndex`) and dummy implementations |
| `server` | Server-side POSIX logic — `MetadataServer<M, D, I>` implements `PosixOps` by composing the three storage traits |
| `client` | FUSE mount layer (`FuseClient`) and client-side `ClientOps` adapter |
| `rpc` | gRPC transport layer — protobuf definitions (`fuse.proto`), TLS, Bearer Token auth |
| `demo` | Single-binary assembly — wires dummy storage into `MetadataServer`, bypasses gRPC for local testing |

### 1.2 Core Design Goals

1. **POSIX Compliance** — Implement all 15 operations defined in the `PosixOps` trait with correct POSIX semantics (atomic rename, nlink maintenance, permission checks, etc.).
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
│           client crate (FuseClient)             │
│  ┌──────────────────────────────────────────┐   │
│  │  fuser::Filesystem impl                  │   │
│  │  Translates FUSE requests → ClientOps    │   │
│  └──────────────────┬───────────────────────┘   │
│                     │ ClientOps trait            │
│  ┌──────────────────▼───────────────────────┐   │
│  │  RPC Client (tonic gRPC stub)            │   │
│  │  Serializes to protobuf, sends over TLS  │   │
│  └──────────────────┬───────────────────────┘   │
└─────────────────────┼───────────────────────────┘
                      │  gRPC / TLS + Bearer Token
                      ▼
┌─────────────────────────────────────────────────┐
│           rpc crate (gRPC Server)               │
│  ┌──────────────────────────────────────────┐   │
│  │  FileSystemService impl                  │   │
│  │  Deserializes protobuf → PosixOps calls  │   │
│  └──────────────────┬───────────────────────┘   │
└─────────────────────┼───────────────────────────┘
                      │  PosixOps trait
                      ▼
┌─────────────────────────────────────────────────┐
│       server crate (MetadataServer<M,D,I>)      │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐  │
│  │MetadataS.│  │DataStore │  │DirectoryIndex│  │
│  │  (M)     │  │  (D)     │  │  (I)         │  │
│  └────┬─────┘  └────┬─────┘  └──────┬───────┘  │
└───────┼──────────────┼───────────────┼──────────┘
        │              │               │
        ▼              ▼               ▼
   ┌─────────┐   ┌──────────┐   ┌──────────────┐
   │ RocksDB │   │ Raw Disk │   │ RocksDB      │
   │ (inode  │   │ data.img │   │ (dir index)  │
   │  attrs) │   │          │   │              │
   └─────────┘   └──────────┘   └──────────────┘
```

### 2.2 Crate Dependency Graph

Dependencies flow **downward only** — no circular dependencies exist.

```
                    ┌──────┐
                    │ core │  (types + traits: PosixOps, ClientOps)
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

The `rpc` crate defines a `FileSystemService` in `fuse.proto` with 15 RPC methods, one for each `PosixOps` method. Each RPC maps 1:1:

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

The server's `MetadataServer<M, D, I>` is generic over three independently replaceable storage backends:

```rust
pub struct MetadataServer<M, D, I>
where
    M: MetadataStore,   // inode attribute CRUD
    D: DataStore,       // file content I/O
    I: DirectoryIndex,  // directory structure
{
    pub metadata: Arc<M>,
    pub data: Arc<D>,
    pub index: Arc<I>,
}
```

**Key decoupling principle:** `MetadataStore` and `DataStore` share **no direct dependency**. They are linked solely by **inode ID** — the metadata engine stores inode attributes keyed by inode ID, and the data engine reads/writes content keyed by the same inode ID. Neither engine needs to know the other's implementation.

| Trait | Current Implementation | Future Alternatives |
|-------|----------------------|-------------------|
| `MetadataStore` | RocksDB | SQLite, TiKV, etcd |
| `DataStore` | `RawDiskDataStore` (local raw file) | S3, Ceph RADOS, MinIO |
| `DirectoryIndex` | RocksDB (same instance as MetadataStore) | In-memory trie, Redis |

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

The `demo` crate compiles client and server into one process, **bypassing gRPC entirely**. The server's `PosixOps` implementation is injected directly into the client's FUSE layer via `Arc<dyn PosixOps>`.

```rust
// demo/src/main.rs — assembly sequence
let metadata = Arc::new(DummyMetadataStore);   // → replace with RocksDB
let index    = Arc::new(DummyDirectoryIndex);  // → replace with RocksDB
let data     = Arc::new(DummyDataStore);       // → replace with RawDiskDataStore

let server = MetadataServer::new(metadata, data, index);
// server implements PosixOps

let client = build_client(Arc::new(server));
// client wraps PosixOps as ClientOps (direct function call, no gRPC)

mount_fuse("/tmp/rucksfs", Arc::new(client));
// FuseClient receives FUSE requests → calls ClientOps → calls PosixOps
```

**Injection chain:** `Concrete Storage Impls` → `MetadataServer<M,D,I>` (implements `PosixOps`) → `build_client()` (wraps as `ClientOps`) → `FuseClient` (implements `fuser::Filesystem`)

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
| `inodes` | Inode attributes | `inode_id` (8 bytes, big-endian u64) | Serialized `InodeValue` (see §3.3) |
| `dir_entries` | Directory children | `parent_inode` (8 BE bytes) + `child_name` (variable, UTF-8) | `child_inode` (8 BE bytes) + `child_kind` (4 BE bytes) |
| `system` | System-level counters | ASCII key string (e.g. `b"next_inode"`) | Value depends on key (e.g. 8 BE bytes for counters) |

**Why three CFs instead of one?**
- `inodes` CF has point-lookup access pattern (get by inode) → optimize with bloom filter.
- `dir_entries` CF has prefix-scan access pattern (list children of a parent) → optimize with prefix extractor.
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

### 3.4 Value Serialization: `InodeValue`

The `inodes` CF value stores a versioned binary structure called `InodeValue`. Serialization uses **`bincode`** (compact binary, Rust-native, deterministic with `fixint_encoding`).

```rust
#[derive(Serialize, Deserialize)]
struct InodeValue {
    version: u8,       // schema version, currently = 1
    // --- FileAttr fields ---
    inode: u64,
    size: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    atime: u64,
    mtime: u64,
    ctime: u64,
    // --- Extended fields ---
    nlink: u32,        // hard link count
}
```

**Version compatibility strategy:**
- The first byte is always the schema version.
- When deserializing, check the version byte first. If `version > CURRENT_VERSION`, return `FsError::InvalidInput`.
- Adding new fields at the end is a forward-compatible change (bump version, older readers skip unknown trailing bytes).

Conversion helpers:
```rust
impl InodeValue {
    fn from_file_attr(attr: &FileAttr, nlink: u32) -> Self {
        InodeValue {
            version: 1,
            inode: attr.inode, size: attr.size, mode: attr.mode,
            uid: attr.uid, gid: attr.gid,
            atime: attr.atime, mtime: attr.mtime, ctime: attr.ctime,
            nlink,
        }
    }

    fn to_file_attr(&self) -> FileAttr {
        FileAttr {
            inode: self.inode, size: self.size, mode: self.mode,
            uid: self.uid, gid: self.gid,
            atime: self.atime, mtime: self.mtime, ctime: self.ctime,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).expect("InodeValue serialization cannot fail")
    }

    fn deserialize(data: &[u8]) -> FsResult<Self> {
        bincode::deserialize(data)
            .map_err(|e| FsError::InvalidInput(format!("corrupt inode value: {e}")))
    }
}
```

### 3.5 Encoding Summary Diagram

```
  inodes CF:
    Key:   [  inode u64 BE  ]  →  Value: [ ver | inode | size | mode | uid | gid | atime | mtime | ctime | nlink ]
                                           1B     8B     8B    4B    4B    4B    8B      8B      8B      4B
                                           Total value: ~57 bytes (bincode)

  dir_entries CF:
    Key:   [ parent_inode u64 BE | child_name UTF-8 ]  →  Value: [ child_inode u64 BE | kind u32 BE ]
            8 bytes                1-255 bytes                     8 bytes              4 bytes

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

The `MetadataStore` trait provides a raw KV interface. The implementation maps each method to a RocksDB operation on the appropriate CF.

```rust
pub struct RocksMetadataStore {
    db: Arc<rocksdb::DB>,
    // Column Family handles, obtained at open time
    cf_inodes: String,      // "inodes"
    cf_dir_entries: String,  // "dir_entries"
    cf_system: String,       // "system"
}
```

**Trait method implementations:**

| Trait Method | RocksDB Operation | CF | Notes |
|---|---|---|---|
| `get(key)` | `db.get_cf(cf, key)` | Determined by key prefix convention (see below) | Returns `Ok(None)` if key not found, not `FsError::NotFound` |
| `put(key, value)` | `db.put_cf(cf, key, value)` | Same | Single-key write; for multi-key atomicity, see §7 WriteBatch |
| `delete(key)` | `db.delete_cf(cf, key)` | Same | Idempotent — deleting a non-existent key is not an error |
| `scan_prefix(prefix)` | `db.prefix_iterator_cf(cf, prefix)` | Same | Iterates until key no longer starts with `prefix`; returns `Vec<(key, value)>` |

**CF routing convention:** The caller (MetadataServer) is responsible for encoding keys that target the correct CF. The `RocksMetadataStore` exposes a helper to select the CF:

```rust
impl RocksMetadataStore {
    /// Determine which CF a key belongs to based on a tag byte prefix.
    /// This is an internal convention — the MetadataServer prepends a tag:
    ///   0x01 → inodes CF
    ///   0x02 → dir_entries CF
    ///   0x03 → system CF
    fn cf_for_key(&self, key: &[u8]) -> &str {
        match key.first() {
            Some(0x01) => &self.cf_inodes,
            Some(0x02) => &self.cf_dir_entries,
            Some(0x03) => &self.cf_system,
            _ => &self.cf_inodes, // fallback
        }
    }

    fn get(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(self.cf_for_key(key))
            .ok_or(FsError::Other("missing CF".into()))?;
        self.db.get_cf(&cf, &key[1..]) // strip tag byte
            .map_err(|e| FsError::Io(e.to_string()))
    }
    // put, delete, scan_prefix follow the same pattern
}
```

> **Alternative design (simpler):** Instead of tag-byte routing, expose separate methods like `get_inode(key)`, `get_dir_entry(key)`, etc. The tag-byte approach is chosen here because it preserves the generic `MetadataStore` trait signature (`get(key: &[u8])`) without modification.

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
| `list_dir(inode)` | `→ FsResult<Vec<DirEntry>>` | Prefix-scan `dir_entries` CF with prefix = `inode.to_be_bytes()` (8 bytes). For each (key, value), decode child name from key\[8..\] and (child\_inode, kind) from value. Also prepend synthetic `.` and `..` entries. |
| `insert_child(parent, name, child_inode, attr)` | `→ FsResult<()>` | Put to `dir_entries` CF: key = `encode_dir_key(parent, name)`, value = `child_inode.to_be_bytes() ++ kind.to_be_bytes()`. The `kind` is extracted from `attr.mode & S_IFMT`. |
| `remove_child(parent, name)` | `→ FsResult<()>` | Delete from `dir_entries` CF: key = `encode_dir_key(parent, name)`. |

**Pseudocode for `list_dir`:**

```rust
fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
    let prefix = inode.to_be_bytes();
    let cf = self.db.cf_handle("dir_entries").unwrap();
    let iter = self.db.prefix_iterator_cf(&cf, &prefix);

    let mut entries = vec![
        DirEntry { name: ".".into(),  inode, kind: S_IFDIR },
        DirEntry { name: "..".into(), inode, kind: S_IFDIR }, // parent lookup deferred
    ];

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
| RocksDB → SQLite for metadata | Implement `MetadataStore` + `DirectoryIndex` for SQLite | `MetadataServer<M,D,I>`, `PosixOps` logic, client, FUSE layer |
| Raw file → S3 for data | Implement `DataStore` for S3 | `MetadataServer<M,D,I>`, `PosixOps` logic, client, FUSE layer |
| Single-node → distributed | Add sharding in `MetadataStore` impl | `PosixOps` semantics remain identical |

The generic parameter constraints enforce this at compile time:
```rust
impl<M, D, I> PosixOps for MetadataServer<M, D, I>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
{ ... }
```

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

---

## 6. POSIX Operations — Detailed Design

Each operation follows a uniform template: **interface signature** → **description** → **preconditions** → **step-by-step implementation** → **CF access pattern** → **error mapping**.

### Common Error Mapping

All operations map `FsError` variants to POSIX errno values:

| `FsError` Variant | POSIX errno | Typical Trigger |
|---|---|---|
| `NotFound` | `ENOENT` | Inode or directory entry does not exist |
| `PermissionDenied` | `EACCES` | Permission check failed |
| `InvalidInput(msg)` | `EINVAL` | Invalid mode, name, offset, etc. |
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

    // Step 2: Fetch child's FileAttr from MetadataStore
    let key = encode_inode_key(child_inode);  // inodes CF
    let value = self.metadata.get(&key)?
        .ok_or(FsError::NotFound)?;  // should not happen if index is consistent

    // Step 3: Deserialize and return
    let inode_val = InodeValue::deserialize(&value)?;
    Ok(inode_val.to_file_attr())
}
```

**CF Access:** `dir_entries` (read) → `inodes` (read). Two point lookups, no writes.

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

**Description:** Retrieve the attributes (metadata) of an inode by its ID.

**Preconditions:**
- `inode` must be a valid allocated inode.

**Implementation Steps:**

```rust
fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
    let key = encode_inode_key(inode);
    let value = self.metadata.get(&key)?
        .ok_or(FsError::NotFound)?;
    let inode_val = InodeValue::deserialize(&value)?;
    Ok(inode_val.to_file_attr())
}
```

**CF Access:** `inodes` (read). Single point lookup.

**Error Mapping:**
| Condition | Error |
|---|---|
| Inode not found | `NotFound` → `ENOENT` |

---

#### 6.1.3 `setattr`

```rust
fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr>;
```

**Description:** Modify attributes of an existing inode. Used for `chmod`, `chown`, `utimes`, `truncate` (size change), etc.

**Preconditions:**
- `inode` must exist.
- Caller must have appropriate permissions (owner or root).

**Implementation Steps:**

```rust
fn setattr(&self, inode: Inode, new_attr: FileAttr) -> FsResult<FileAttr> {
    let key = encode_inode_key(inode);

    // Step 1: Read current attributes (read-modify-write)
    let old_value = self.metadata.get(&key)?
        .ok_or(FsError::NotFound)?;
    let mut inode_val = InodeValue::deserialize(&old_value)?;

    // Step 2: Permission check
    //   - Only owner (uid match) or root (uid=0) can setattr
    //   - See §8 Security for detailed logic

    // Step 3: Apply changes selectively
    //   The caller sets only the fields they want to change.
    //   Fields with value 0 / unchanged are skipped (convention).
    if new_attr.mode != 0   { inode_val.mode  = new_attr.mode; }
    if new_attr.uid  != 0   { inode_val.uid   = new_attr.uid; }
    if new_attr.gid  != 0   { inode_val.gid   = new_attr.gid; }
    if new_attr.atime != 0  { inode_val.atime = new_attr.atime; }
    if new_attr.mtime != 0  { inode_val.mtime = new_attr.mtime; }

    // Step 4: Handle size change (truncate)
    if new_attr.size != inode_val.size {
        // Delegate to DataStore for content truncation
        block_on(self.data.truncate(inode, new_attr.size))?;
        inode_val.size = new_attr.size;
    }

    // Step 5: Update ctime (always changes on setattr)
    inode_val.ctime = now();

    // Step 6: Write back
    self.metadata.put(&key, &inode_val.serialize())?;

    Ok(inode_val.to_file_attr())
}
```

**CF Access:** `inodes` (read + write). Single key read-modify-write.

**TOCTOU mitigation:** In a concurrent environment, the read-modify-write must be protected. Options:
1. **RocksDB Merge Operator** — define a custom merge that applies delta changes atomically.
2. **Optimistic Transaction** — use `TransactionDB::GetForUpdate` to detect write conflicts.
3. **Application-level mutex** — per-inode `RwLock` (simplest for demo). See §7.

**Error Mapping:**
| Condition | Error |
|---|---|
| Inode not found | `NotFound` → `ENOENT` |
| Caller is not owner and not root | `PermissionDenied` → `EACCES` |

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
    // Step 1: Verify parent is a directory and check permissions
    let parent_attr = self.getattr(parent)?;
    check_write_permission(&parent_attr)?;

    // Step 2: Check name does not already exist
    if self.index.resolve_path(parent, name)?.is_some() {
        return Err(FsError::Other("file exists".into()));  // EEXIST
    }

    // Step 3: Allocate new inode
    let new_inode = self.allocator.alloc();

    // Step 4: Construct FileAttr for new file
    let now = now();
    let new_attr = FileAttr {
        inode: new_inode,
        size: 0,
        mode: S_IFREG | (mode & 0o7777),  // regular file + permission bits
        uid: caller_uid(), gid: caller_gid(),
        atime: now, mtime: now, ctime: now,
    };
    let inode_val = InodeValue::from_file_attr(&new_attr, 1); // nlink=1

    // Step 5: Atomic WriteBatch (see §7 for details)
    //   a. Put new inode to inodes CF
    //   b. Insert dir entry to dir_entries CF
    //   c. Update parent mtime/ctime in inodes CF
    //   d. Persist inode allocator counter to system CF
    let mut batch = WriteBatch::default();
    batch.put_cf(cf_inodes, encode_inode_key(new_inode), inode_val.serialize());
    batch.put_cf(cf_dir_entries, encode_dir_key(parent, name), encode_dir_value(new_inode, S_IFREG));
    batch.put_cf(cf_inodes, encode_inode_key(parent), updated_parent_attr.serialize());
    batch.put_cf(cf_system, b"next_inode", self.allocator.next.load().to_be_bytes());
    db.write(batch)?;

    Ok(new_attr)
}
```

**CF Access:** `inodes` (read parent + write new + write parent) → `dir_entries` (read check + write) → `system` (write counter).

**Atomicity:** All writes bundled in a single `WriteBatch`. If the write fails, no partial state is created.

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
    // Steps 1-2: Same as create (verify parent, check name doesn't exist)

    // Step 3: Allocate new inode
    let new_inode = self.allocator.alloc();

    // Step 4: Construct FileAttr for new directory
    let new_attr = FileAttr {
        inode: new_inode,
        size: 0,
        mode: S_IFDIR | (mode & 0o7777),
        uid: caller_uid(), gid: caller_gid(),
        atime: now, mtime: now, ctime: now,
    };
    let inode_val = InodeValue::from_file_attr(&new_attr, 2);  // nlink=2

    // Step 5: Update parent: nlink += 1, mtime = now, ctime = now
    let mut parent_val = load_inode_value(parent)?;
    parent_val.nlink += 1;
    parent_val.mtime = now;
    parent_val.ctime = now;

    // Step 6: Atomic WriteBatch
    let mut batch = WriteBatch::default();
    batch.put_cf(cf_inodes, encode_inode_key(new_inode), inode_val.serialize());
    batch.put_cf(cf_dir_entries, encode_dir_key(parent, name), encode_dir_value(new_inode, S_IFDIR));
    batch.put_cf(cf_inodes, encode_inode_key(parent), parent_val.serialize());
    batch.put_cf(cf_system, b"next_inode", allocator_next_bytes);
    db.write(batch)?;

    Ok(new_attr)
}
```

**CF Access:** Same as `create`, plus parent nlink update.

---

#### 6.2.4 `unlink`

```rust
fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
```

**Description:** Remove a directory entry for a regular file. If no more references exist (nlink reaches 0), the inode and its data content are eligible for deletion.

**Implementation Steps:**

```rust
fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
    // Step 1: Resolve the target inode
    let child_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;

    // Step 2: Verify target is NOT a directory (use rmdir for directories)
    let child_attr = self.getattr(child_inode)?;
    if child_attr.mode & S_IFDIR != 0 {
        return Err(FsError::InvalidInput("is a directory".into()));  // EISDIR
    }

    // Step 3: Load and decrement nlink
    let mut child_val = load_inode_value(child_inode)?;
    child_val.nlink -= 1;
    child_val.ctime = now();

    // Step 4: Update parent directory mtime/ctime
    let mut parent_val = load_inode_value(parent)?;
    parent_val.mtime = now();
    parent_val.ctime = now();

    // Step 5: Atomic WriteBatch
    let mut batch = WriteBatch::default();
    batch.delete_cf(cf_dir_entries, encode_dir_key(parent, name));
    batch.put_cf(cf_inodes, encode_inode_key(parent), parent_val.serialize());

    if child_val.nlink == 0 {
        // No more references — delete inode and content
        batch.delete_cf(cf_inodes, encode_inode_key(child_inode));
        // Data cleanup: either inline or deferred
        // For simplicity, inline cleanup:
        //   DataStore content becomes unreachable (inode slot can be reused)
        //   In demo mode with no inode reclamation, the data region is simply abandoned
    } else {
        // Still has references — keep inode with updated nlink
        batch.put_cf(cf_inodes, encode_inode_key(child_inode), child_val.serialize());
    }

    db.write(batch)?;
    Ok(())
}
```

**Deferred deletion (open-file case):** If the file is currently open (tracked by an in-memory `HashMap<Inode, u32>` of open handle counts), the actual inode + data deletion is deferred until the last `flush`/`close`. The dir entry is removed immediately (POSIX: unlinked files remain accessible via open file descriptors).

**CF Access:** `dir_entries` (read + delete) → `inodes` (read child + read parent + write both).

---

#### 6.2.5 `rmdir`

```rust
fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
```

**Description:** Remove an empty directory.

**Implementation Steps:**

```rust
fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
    // Step 1: Resolve the target
    let child_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;

    // Step 2: Verify target IS a directory
    let child_attr = self.getattr(child_inode)?;
    if child_attr.mode & S_IFDIR == 0 {
        return Err(FsError::InvalidInput("not a directory".into()));  // ENOTDIR
    }

    // Step 3: Check directory is empty
    let children = self.index.list_dir(child_inode)?;
    // list_dir returns at least "." and ".." — if len > 2, dir is not empty
    if children.len() > 2 {
        return Err(FsError::Other("directory not empty".into()));  // ENOTEMPTY
    }

    // Step 4: Update parent nlink -= 1, mtime, ctime
    let mut parent_val = load_inode_value(parent)?;
    parent_val.nlink -= 1;
    parent_val.mtime = now();
    parent_val.ctime = now();

    // Step 5: Atomic WriteBatch
    let mut batch = WriteBatch::default();
    batch.delete_cf(cf_dir_entries, encode_dir_key(parent, name));
    batch.delete_cf(cf_inodes, encode_inode_key(child_inode));
    batch.put_cf(cf_inodes, encode_inode_key(parent), parent_val.serialize());
    db.write(batch)?;

    Ok(())
}
```

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
    // Step 1: Resolve source
    let src_inode = self.index.resolve_path(parent, name)?
        .ok_or(FsError::NotFound)?;
    let src_val = load_inode_value(src_inode)?;
    let src_is_dir = src_val.mode & S_IFDIR != 0;

    // Step 2: Check if target already exists
    let maybe_target = self.index.resolve_path(new_parent, new_name)?;

    let mut batch = WriteBatch::default();
    let now = now();

    // Step 3: Handle existing target (POSIX atomic replacement)
    if let Some(target_inode) = maybe_target {
        let target_val = load_inode_value(target_inode)?;
        let target_is_dir = target_val.mode & S_IFDIR != 0;

        // POSIX constraint: cannot replace dir with non-dir and vice versa
        if src_is_dir && !target_is_dir {
            return Err(FsError::InvalidInput("cannot replace file with directory".into()));
            // ENOTDIR
        }
        if !src_is_dir && target_is_dir {
            return Err(FsError::InvalidInput("cannot replace directory with file".into()));
            // EISDIR
        }

        // If target is a directory, it must be empty
        if target_is_dir {
            let children = self.index.list_dir(target_inode)?;
            if children.len() > 2 {
                return Err(FsError::Other("target directory not empty".into()));
                // ENOTEMPTY
            }
        }

        // Delete the target inode
        batch.delete_cf(cf_inodes, encode_inode_key(target_inode));

        // If target was a directory, new_parent.nlink -= 1
        if target_is_dir {
            let mut np_val = load_inode_value(new_parent)?;
            np_val.nlink -= 1;
            np_val.mtime = now;
            np_val.ctime = now;
            batch.put_cf(cf_inodes, encode_inode_key(new_parent), np_val.serialize());
        }
    }

    // Step 4: Remove old dir entry, insert new dir entry
    batch.delete_cf(cf_dir_entries, encode_dir_key(parent, name));
    batch.put_cf(cf_dir_entries,
        encode_dir_key(new_parent, new_name),
        encode_dir_value(src_inode, if src_is_dir { S_IFDIR } else { S_IFREG }));

    // Step 5: Handle cross-directory nlink updates (for directories only)
    if src_is_dir && parent != new_parent {
        // Old parent loses a ".." reference: nlink -= 1
        let mut old_parent_val = load_inode_value(parent)?;
        old_parent_val.nlink -= 1;
        old_parent_val.mtime = now;
        old_parent_val.ctime = now;
        batch.put_cf(cf_inodes, encode_inode_key(parent), old_parent_val.serialize());

        // New parent gains a ".." reference: nlink += 1
        // (unless already updated in Step 3 for target replacement)
        if maybe_target.is_none() || !load_inode_value(maybe_target.unwrap())?.is_dir() {
            let mut new_parent_val = load_inode_value(new_parent)?;
            new_parent_val.nlink += 1;
            new_parent_val.mtime = now;
            new_parent_val.ctime = now;
            batch.put_cf(cf_inodes, encode_inode_key(new_parent), new_parent_val.serialize());
        }
    }

    // Step 6: Update parent mtime/ctime (same-directory rename)
    if parent == new_parent {
        let mut p_val = load_inode_value(parent)?;
        p_val.mtime = now;
        p_val.ctime = now;
        batch.put_cf(cf_inodes, encode_inode_key(parent), p_val.serialize());
    }

    // Step 7: Update source inode ctime
    let mut src_updated = src_val.clone();
    src_updated.ctime = now;
    batch.put_cf(cf_inodes, encode_inode_key(src_inode), src_updated.serialize());

    // Step 8: Atomic commit
    db.write(batch)?;
    Ok(())
}
```

**CF Access:** `dir_entries` (read src + read dst + delete src + put dst) → `inodes` (read/write multiple inodes).

**WriteBatch Contents Summary (worst case — cross-dir rename replacing existing dir):**

| Operation | CF | Key |
|---|---|---|
| Delete old entry | `dir_entries` | `(parent, name)` |
| Insert new entry | `dir_entries` | `(new_parent, new_name)` |
| Delete replaced target inode | `inodes` | `target_inode` |
| Update source inode (ctime) | `inodes` | `src_inode` |
| Update old parent (nlink-1, mtime) | `inodes` | `parent` |
| Update new parent (nlink+1, mtime) | `inodes` | `new_parent` |

All in a single atomic `WriteBatch`.

---

### 6.3 Data Operations

#### 6.3.1 `open`

```rust
fn open(&self, inode: Inode, flags: u32) -> FsResult<u64>;
```

**Description:** Open a file and return a file handle. In a stateless server design, this mainly validates the inode and flags.

**Implementation Steps:**

```rust
fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
    // Step 1: Verify inode exists
    let attr = self.getattr(inode)?;

    // Step 2: Verify it's a regular file (not a directory)
    if attr.mode & S_IFDIR != 0 {
        return Err(FsError::InvalidInput("is a directory".into()));  // EISDIR
    }

    // Step 3: Permission check based on flags
    //   O_RDONLY (0) → check read permission
    //   O_WRONLY (1) → check write permission
    //   O_RDWR   (2) → check both
    check_open_permission(&attr, flags)?;

    // Step 4: Generate file handle
    //   For stateless demo: return inode as handle (1:1 mapping)
    //   For stateful impl: allocate from handle table, track open count
    let handle = inode;  // simplest approach

    // Step 5: Track open handle (for deferred unlink)
    //   self.open_handles.entry(inode).or_insert(0) += 1;

    Ok(handle)
}
```

**CF Access:** `inodes` (read). No writes.

**File Handle Strategy (Demo):** Use inode ID directly as the handle. This is stateless and simple. A production implementation would maintain a `HashMap<u64, OpenFileState>` for per-handle state (cursor position, buffered writes, etc.).

---

#### 6.3.2 `read`

```rust
fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
```

**Description:** Read file content at the given offset.

**Implementation Steps:**

```rust
fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
    // Step 1: Verify inode exists and get current size
    let attr = self.getattr(inode)?;

    // Step 2: Boundary check
    if offset >= attr.size {
        return Ok(vec![]);  // read past EOF → return empty
    }
    let actual_size = std::cmp::min(size as u64, attr.size - offset) as u32;

    // Step 3: Delegate to DataStore
    let data = block_on(self.data.read_at(inode, offset, actual_size))?;

    // Step 4: Update atime (optional — many modern FSes mount with noatime)
    //   Skip for demo to avoid write amplification on every read

    Ok(data)
}
```

**CF Access:** `inodes` (read for size check) → DataStore (read_at).

**Note:** The `read_at` call goes to `RawDiskDataStore`, which computes `disk_offset = inode * max_file_size + offset` and calls `pread`.

---

#### 6.3.3 `write`

```rust
fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32>;
```

**Description:** Write data to a file at the given offset. Updates file size and mtime.

**Implementation Steps:**

```rust
fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
    // Step 1: Verify inode exists
    let attr = self.getattr(inode)?;

    // Step 2: Delegate to DataStore
    let bytes_written = block_on(self.data.write_at(inode, offset, data))?;

    // Step 3: Update metadata (size and mtime)
    let new_size = std::cmp::max(attr.size, offset + bytes_written as u64);
    let mut inode_val = load_inode_value(inode)?;
    inode_val.size = new_size;
    inode_val.mtime = now();
    inode_val.ctime = now();
    self.metadata.put(&encode_inode_key(inode), &inode_val.serialize())?;

    Ok(bytes_written)
}
```

**CF Access:** `inodes` (read + write) → DataStore (write_at).

**Consistency note:** The data write and metadata update are **not** in the same transaction (DataStore is a separate engine). If the process crashes after data write but before metadata update:
- File content is written but size/mtime is stale.
- On recovery, the metadata still shows old size → reads are bounded to old size → extra data beyond old size is invisible but harmless.
- This is acceptable for demo. Production systems would use a write journal (see §9).

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
| `create` | Put new inode + Put dir entry + Update parent mtime + Persist allocator | `inodes`, `dir_entries`, `system` |
| `mkdir` | Put new inode + Put dir entry + Update parent (nlink+1, mtime) + Persist allocator | `inodes`, `dir_entries`, `system` |
| `unlink` | Delete dir entry + Update/Delete child inode (nlink) + Update parent mtime | `inodes`, `dir_entries` |
| `rmdir` | Delete dir entry + Delete child inode + Update parent (nlink-1, mtime) | `inodes`, `dir_entries` |
| `rename` | Delete old entry + Put new entry + Update/Delete target + Update parents + Update source ctime | `inodes`, `dir_entries` |

Operations that are single-key writes (`setattr`, `write` metadata update) do not require WriteBatch but still benefit from RocksDB's WAL durability.

### 7.3 Concurrency Control Strategy

The demo implementation uses a **coarse-grained approach** suitable for a single-server, moderate-concurrency scenario:

#### Strategy: Per-Directory Mutex

```rust
pub struct MetadataServer<M, D, I> {
    // ...
    dir_locks: DashMap<Inode, Arc<Mutex<()>>>,  // per-directory lock
}
```

**Lock acquisition rules:**
1. Any operation that **modifies** a directory (create, mkdir, unlink, rmdir, rename) must hold the lock for that directory's inode.
2. Read-only directory operations (readdir, lookup) do **not** acquire locks — they rely on RocksDB's consistent snapshots.
3. For cross-directory `rename`, acquire locks for **both** directories in inode-ID order to prevent deadlock.

```rust
// rename: deadlock-free lock ordering
let (first, second) = if parent < new_parent {
    (parent, new_parent)
} else {
    (new_parent, parent)
};
let _g1 = self.dir_locks.entry(first).or_default().lock();
let _g2 = self.dir_locks.entry(second).or_default().lock();
```

#### Alternative: RocksDB OptimisticTransactionDB (Future)

For higher concurrency, replace `WriteBatch` with `OptimisticTransactionDB`:
- Use `GetForUpdate` to detect read-write conflicts.
- On conflict, the transaction returns `Status::Busy` → retry with exponential backoff.
- No external locking needed.

### 7.4 TOCTOU Prevention

**Time-of-Check to Time-of-Use** race conditions occur when a value read during validation is stale by the time it is used. Specific scenarios in rucksfs:

| Scenario | Risk | Mitigation |
|---|---|---|
| `create`: check name doesn't exist, then insert | Another thread creates same name between check and insert | Per-directory mutex serializes all mutations |
| `setattr`: read attr, modify, write back | Another thread modifies attr between read and write | Per-inode mutex for concurrent setattr |
| `rmdir`: check dir is empty, then delete | Another thread creates child between check and delete | Per-directory mutex covers both check and delete |
| `rename`: check target exists, then replace | Another thread modifies target between check and replace | Lock both source and destination directories |

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
         Pass (uid, gid) to PosixOps methods for permission checks
```

**In demo mode (single binary):** Authentication is bypassed. The caller identity is hardcoded (e.g., uid=1000, gid=1000) or derived from the process's real UID.

**In distributed mode:** The gRPC interceptor (`tonic::service::interceptor`) validates the Bearer Token before any `PosixOps` method is invoked. The token is verified using constant-time comparison to prevent timing attacks.

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
| **bincode** | A compact binary serialization format for Rust. Used to encode `InodeValue` structures for storage in RocksDB. |
| **fdatasync** | A POSIX system call that flushes file data to disk without flushing metadata (more efficient than `fsync` when only data durability is needed). |
