# RucksFS

A modular, trait-based distributed file system written in Rust, backed by **RocksDB** for metadata and **RawDisk** for file data. RucksFS exposes a standard POSIX mount via FUSE, with a clean separation between metadata, data, and client layers that enables both single-binary embedded deployment and multi-node distributed deployment.

This repository also serves as the **supporting repository** for the accompanying B.Eng. thesis *"基于 KV 存储管理文件系统元数据"*. All benchmark raw data, orchestration scripts, reports, and the thesis LaTeX source are versioned in-tree.

---

## Highlights

- **Full POSIX metadata semantics** — `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `readdir`, `getattr`, `setattr`, `statfs`, `flush`, `fsync`
- **Edge-centric key encoding** — directory entries keyed as `<parent_inode_big_endian, child_name>` turn tree operations into constant-time edge manipulations
- **Delta-based metadata updates** — parent-directory attribute changes are append-only DeltaOps folded lazily by a background compactor, eliminating hot-parent write amplification
- **Pessimistic Concurrency Control** — all multi-key metadata operations (rename, rmdir, unlink) run inside RocksDB pessimistic transactions with sorted-inode locking to prevent deadlocks
- **FUSE mount with `default_permissions`** — delegates POSIX permission checks to the kernel VFS
- **gRPC-based distributed mode** — `Client` / `MetadataServer` / `DataServer` run on separate nodes
- **Embedded mode** — all three in one process for development and single-node deployments
- **Interactive REPL + auto-demo** — `rucksfs` binary ships three modes (mount / REPL / demo) behind one CLI

---

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                     rucksfs (binary)                           │
│  CLI: auto-demo │ interactive REPL │ FUSE mount                │
├────────────────────────────────────────────────────────────────┤
│                   rucksfs-client                               │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ VfsCore (routes metadata ↔ data)                         │  │
│  │ ├─ EmbeddedClient   (in-process, same-binary deployment) │  │
│  │ └─ RemoteClient     (gRPC to MDS/DS, distributed mode)   │  │
│  │ FuseClient          (fuser 0.x adapter, Linux only)      │  │
│  └─────────────┬─────────────────┬────────────────────────┘    │
│                MetadataOps        DataOps                      │
├───────────────┬──────────────────┬─────────────────────────────┤
│ rucksfs-server│                  │ rucksfs-dataserver          │
│ MetadataServer│                  │ DataServer                  │
│ (namespace,   │                  │ (file I/O,                  │
│  attributes,  │                  │  block allocation,          │
│  delta engine,│                  │  RawDiskDataStore)          │
│  PCC txns)    │                  │                             │
├───────────────┴──────────────────┴─────────────────────────────┤
│                  rucksfs-storage                               │
│  RocksMetadataStore │ RocksDirectoryIndex │ RocksDeltaStore    │
│  + RawDiskDataStore                                            │
├────────────────────────────────────────────────────────────────┤
│                   rucksfs-core                                 │
│  Traits: MetadataOps, DataOps, VfsOps                          │
│  Types:  FileAttr, DirEntry, StatFs, FsError                   │
└────────────────────────────────────────────────────────────────┘
```

---

## Repository Layout

This repo contains three kinds of content: **source code**, **thesis**, and **benchmark artifacts**.

### Source Code (the implementation)

| Directory | Description |
|---|---|
| `core/` | Shared types and trait definitions (`MetadataOps`, `DataOps`, `VfsOps`, `FileAttr`, `FsError`) |
| `storage/` | RocksDB- and RawDisk-backed implementations of the storage traits; key encoding |
| `server/` | `MetadataServer`: namespace engine, delta compaction worker, PCC transaction layer |
| `dataserver/` | `DataServer`: file data I/O engine |
| `client/` | `VfsCore` router, `EmbeddedClient`, `RemoteClient`, `FuseClient` |
| `rpc/` | gRPC layer (tonic + Prost), protocol buffers, client connection pool |
| `demo/` | Single-binary CLI entry point |
| `scripts/` | Local E2E test helpers (`e2e_fuse_test.sh`, etc.) |

### Thesis (毕业论文)

| Path | Description |
|---|---|
| `docs/thesis-template/` | XeLaTeX source of the thesis |
| `docs/thesis-template/main.tex` | Top-level document; run `latexmk -xelatex main.tex` to build |
| `docs/thesis-template/body/` | Chapter files: `abstract-ch.tex`, `abstract-en.tex`, `introduction.tex`, `related_works.tex`, `method.tex`, `experiments.tex`, `conclusion.tex` |
| `docs/thesis-template/ref.bib` | References |
| `docs/thesis-template/FUTURE_TESTS.md` | Known gaps and suggested follow-up experiments |

### Supporting Documentation

| Document | Purpose |
|---|---|
| `docs/design.md` | Full system design (≈120 KB; consult by section, do NOT read whole) |
| `docs/guide.md` | Deployment, mount, and operational guide |
| `docs/standalone-analysis.md` | Architecture comparison with JuiceFS / TableFS |
| `docs/metadata-kv-research.md` | Literature survey on KV-backed metadata |
| `docs/TODO.md` | Active development task list |

### Benchmark Artifacts

All measurement data cited in the thesis lives under the following paths. Numbers in the thesis can be regenerated from these inputs using the scripts in `testing/round3_scripts/`.

| Path | Purpose |
|---|---|
| **`testing/round3_scripts/`** | Final measurement orchestration: `master-orchestrator.sh`, `run-mdtest.sh`, `switch-sut.sh`, `setup-ssh-mesh.sh`, `parse-results.py`. These drove the 5-concurrency-level × 3-repetition mdtest runs. |
| **`testing/round3_results/phase1/`** | Per-SUT raw mdtest output (RucksFS-Delta, RucksFS-NoDelta) across `N ∈ {8, 16, 32, 64, 96, 128, 192}` × 3 runs. |
| **`testing/bench-v2/`** | Final per-concurrency-level measurements used in thesis §4.3 and §4.4: `results-v2-n2/n8/n32/n64/n96/`. Each subdirectory contains raw mdtest logs per SUT (RucksFS-Delta, RucksFS-NoDelta, NFS, JuiceFS+TiKV) plus a `summary.csv`. `orchestrator.sh` orchestrates the full sweep. |
| **`testing/results/pjdfstest_final_20260424.txt`** | pjdfstest raw output (398 cases) used in thesis §4.2 (POSIX compliance). |
| **`docs/benchmark-report-v2.md`** | Human-readable write-up of the benchmark results. Mirrors the data presented in thesis §4.3–§4.4. |
| **`docs/benchmark-plan-v2.md`** | Pre-registered experiment protocol (cluster spec, SUT configuration, parameter ranges). |
| `docs/benchmark-archive-legacy.md` | Explains which older measurement rounds were superseded and why. For context only; not a thesis data source. |
| `docs/round3-unified-benchmark-plan.md` | Motivation for moving from Round 2 to the Round 3 distributed topology. |
| `docs/round3-phase1-findings.md` | Findings log from the Phase 1 runs of Round 3. |
| `docs/delta-bottleneck-diagnosis.md` | Root-cause analysis of the apparent FUSE+gRPC ceiling; feeds into thesis §4.4. |
| `docs/commlayer-optim-plan.md` | Communication-layer optimization plan (client connection pool, merged create+open RPC). |

### Infrastructure

| Path | Purpose |
|---|---|
| `infra/tencent-bench/` | Terraform config for the benchmark cluster (1×64C server + N×2C clients on Tencent Cloud Hong Kong). `variables.tf` is checked in; `terraform.tfvars`, `*.tfstate`, and `.pem` keys are ignored. |
| `infra/tencent-grpc-direct/` | Terraform config for the gRPC-direct diagnostic cluster used in `docs/delta-bottleneck-diagnosis.md`. |

---

## Quick Start

### Build

```bash
cargo build --workspace                           # build all crates
cargo build --release -p rucksfs --features rocksdb   # release binary
```

### Run (single-binary mode)

```bash
# Automatic demo (data at ~/.rucksfs)
cargo run -p rucksfs

# Interactive REPL
cargo run -p rucksfs -- --interactive

# Custom data directory
cargo run -p rucksfs -- --data-dir /tmp/my-rucksfs
```

### FUSE Mount (Linux only)

```bash
sudo apt-get install libfuse-dev fuse               # Debian/Ubuntu
sudo sh -c 'echo user_allow_other >> /etc/fuse.conf'

# Mount
cargo run -p rucksfs -- --mount /mnt/rucksfs

# Use with standard tools
echo "hello" > /mnt/rucksfs/test.txt
cat /mnt/rucksfs/test.txt

# Unmount
fusermount -u /mnt/rucksfs
```

### Distributed Mode

See `docs/guide.md` for the full three-process deployment (MetadataServer + DataServer + FUSE client over gRPC).

---

## Testing

```bash
cargo test --workspace                 # ~192 unit/integration tests
./scripts/e2e_fuse_test.sh             # local FUSE end-to-end
```

Remote (distributed) end-to-end tests and the benchmark pipeline live under `testing/round3_scripts/` and `testing/bench-v2/`. See `docs/guide.md` and `docs/benchmark-plan-v2.md` for the full protocol.

---

## Reproducing the Thesis Results

The high-level reproduction path:

1. Provision the cluster with `infra/tencent-bench/` (1 × SA5.16XLARGE256 server + up to 96 × SA5.MEDIUM2 clients in the same Tencent Cloud VPC).
2. Build the two RucksFS variants with `cargo build --release -p rucksfs` and `cargo build --release -p rucksfs --features no_delta` (ablation build).
3. Run `testing/round3_scripts/master-orchestrator.sh` to drive the full sweep (five `N` values × three repetitions × four SUTs × two sharing modes).
4. Raw per-run mdtest output lands in `testing/bench-v2/results-v2-nN/`; summaries are aggregated into `summary.csv` per directory.
5. pjdfstest is driven separately; the archived run is `testing/results/pjdfstest_final_20260424.txt`.
6. Thesis numbers in chapter 4 (§4.2–§4.5) can be traced back to these CSV / raw-text files one-to-one.

`docs/benchmark-plan-v2.md` documents the full protocol; `docs/benchmark-report-v2.md` is the narrative report that parallels thesis §4.

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| **RocksDB for metadata** | LSM-tree batch-append throughput dominates B-tree in-place rewrites for metadata-heavy workloads; Column Families isolate inodes from directory entries from deltas |
| **Edge-centric key encoding** | `<parent_inode_be, child_name>` makes directory listing a prefix scan and rename a constant-time two-edge swap; contrast with full-path keys that cost O(subtree) on rename |
| **Delta-based parent updates** | Append-only DeltaOps eliminate read-modify-write on hot parent inodes; background `DeltaCompactionWorker` folds deltas at a threshold of 32 |
| **Pessimistic transactions with sorted locks** | PCC puts the TOCTOU check inside the transaction; inodes are locked in sorted order to prevent deadlocks across rename / unlink |
| **FUSE `default_permissions`** | POSIX permission checks are delegated to the kernel VFS, keeping MetadataServer focused on namespace logic |
| **DataServer as a minimum viable implementation** | `RawDiskDataStore` uses the fixed formula `inode × max_file_size + offset`; space reclamation is deliberately deferred. This is acknowledged in thesis §4.1.4 as a constraint on the performance comparison |

---

## License

MIT — see [`LICENSE`](./LICENSE).
