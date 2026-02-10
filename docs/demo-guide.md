# RucksFS Demo Guide

This guide walks you through building, running, and exploring the RucksFS demo — a single-binary program that embeds both the metadata server and the client in one process.

---

## Prerequisites

| Requirement | Version | Notes |
|---|---|---|
| Rust toolchain | ≥ 1.70 | Install via [rustup](https://rustup.rs/) |
| RocksDB (optional) | — | Only needed for `--persist` mode; the `rocksdb` crate builds it from source automatically |
| FUSE / libfuse-dev (optional) | — | Only needed for `--mount` mode on Linux |

### Platform Notes

- **macOS / Windows**: The demo runs in auto-demo and interactive modes. FUSE mount is Linux-only.
- **Linux**: All three modes (auto-demo, interactive, FUSE mount) are available.

---

## Quick Start (≤ 5 Steps)

```bash
# 1. Clone the repository
git clone https://github.com/csjgg/rucksfs.git
cd rucksfs

# 2. Build the demo (in-memory mode, no optional dependencies)
cargo build -p rucksfs-demo

# 3. Run the automatic demo
cargo run -p rucksfs-demo

# 4. (Optional) Run the interactive REPL
cargo run -p rucksfs-demo -- --interactive

# 5. (Optional) Run with persistent storage (requires RocksDB)
cargo run -p rucksfs-demo --features rocksdb -- --persist /tmp/rucksfs-data
```

---

## Command-Line Options

```
rucksfs-demo [OPTIONS]

Options:
    --interactive          Enter interactive REPL mode
    --mount <MOUNTPOINT>   Mount as FUSE filesystem (Linux only)
    --persist <DIR>        Use persistent RocksDB + RawDisk storage
    -h, --help             Print help
    -V, --version          Print version
```

### Mode Priority

| Flags | Mode |
|---|---|
| (none) | Automatic demo — runs 10 POSIX operations sequentially |
| `--interactive` | Interactive REPL shell |
| `--mount /mnt/rucksfs` | FUSE mount (Linux only) |

The `--persist` flag can be combined with any mode to switch from in-memory storage to RocksDB + RawDisk.

---

## Automatic Demo Mode (Default)

Simply run:

```bash
cargo run -p rucksfs-demo
```

This executes 10 steps:

| Step | Operation | Description |
|---:|---|---|
| 1 | `mkdir /mydir` | Create a directory |
| 2 | `create /mydir/hello.txt` | Create a file |
| 3 | `write` | Write "Hello, RucksFS!" to the file |
| 4 | `read` | Read back the file content |
| 5 | `readdir /mydir` | List directory entries |
| 6 | `rename` | Rename `hello.txt` → `greeting.txt` |
| 7 | `getattr` | Get attributes of the renamed file |
| 8 | `unlink` | Delete the file |
| 9 | `rmdir` | Remove the directory |
| 10 | `statfs` | Query filesystem statistics |

Each step prints `✓` on success or `✗` on failure.

---

## Interactive REPL Mode

```bash
cargo run -p rucksfs-demo -- --interactive
```

### Available Commands

| Command | Description | Example |
|---|---|---|
| `ls [path]` | List directory contents (default: `/`) | `ls /mydir` |
| `mkdir <path>` | Create a directory | `mkdir /projects` |
| `touch <path>` | Create an empty file | `touch /projects/main.rs` |
| `write <path> <text>` | Write text to a file | `write /projects/main.rs fn main() {}` |
| `cat <path>` | Read and display file content | `cat /projects/main.rs` |
| `rm <path>` | Remove a file | `rm /projects/main.rs` |
| `rmdir <path>` | Remove an empty directory | `rmdir /projects` |
| `mv <src> <dst>` | Rename/move a file or directory | `mv /old.txt /new.txt` |
| `stat <path>` | Show file/directory attributes | `stat /mydir` |
| `statfs` | Show filesystem statistics | `statfs` |
| `help` | Show help message | `help` |
| `exit` | Exit the REPL | `exit` |

### Path Resolution

- All paths are resolved from root `/` (inode 1).
- Relative paths (without leading `/`) are also supported and resolved from root.
- Nested paths like `/a/b/c` are walked component-by-component via `lookup`.

### Example Session

```
rucksfs> mkdir /docs
  created directory inode=2
rucksfs> touch /docs/readme.txt
  created file inode=3
rucksfs> write /docs/readme.txt Hello World
  wrote 11 bytes
rucksfs> cat /docs/readme.txt
Hello World
rucksfs> ls /docs
  - readme.txt (inode=3)
rucksfs> stat /docs/readme.txt
  inode:  3
  type:   file
  size:   11 bytes
  mode:   0o100644
  nlink:  1
  uid:    0
  gid:    0
rucksfs> mv /docs/readme.txt /docs/README.txt
  renamed
rucksfs> rm /docs/README.txt
  removed
rucksfs> rmdir /docs
  removed directory
rucksfs> exit
Goodbye!
```

---

## Persistent Storage Mode

By default, the demo uses in-memory storage — data is lost when the process exits.
To persist data across restarts, use the `--persist` flag with the `rocksdb` feature:

```bash
# Build and run with persistence
cargo run -p rucksfs-demo --features rocksdb -- --persist /tmp/rucksfs-data

# Or with interactive mode
cargo run -p rucksfs-demo --features rocksdb -- --interactive --persist /tmp/rucksfs-data
```

This creates two files in the specified directory:
- `metadata.db/` — RocksDB database for inode metadata and directory entries
- `data.raw` — Raw file backing store for file content

Data survives process restarts as long as you point to the same directory.

---

## FUSE Mount Mode (Linux Only)

> **Requirements**: Install FUSE development libraries first:
> ```bash
> # Debian/Ubuntu
> sudo apt-get install libfuse-dev fuse
>
> # Fedora/RHEL
> sudo dnf install fuse-devel fuse
> ```

```bash
# Create mountpoint and mount
sudo mkdir -p /mnt/rucksfs
cargo run -p rucksfs-demo -- --mount /mnt/rucksfs

# In another terminal, use standard tools:
ls /mnt/rucksfs
mkdir /mnt/rucksfs/test
echo "hello" > /mnt/rucksfs/test/hello.txt
cat /mnt/rucksfs/test/hello.txt

# Unmount
fusermount -u /mnt/rucksfs
```

On non-Linux platforms, `--mount` prints a warning and falls back to the auto-demo.

---

## Running Tests

```bash
# Run all tests (in-memory only)
cargo test --workspace

# Run demo integration tests only
cargo test -p rucksfs-demo

# Run with RocksDB persistence tests
cargo test -p rucksfs-demo --features rocksdb

# Run all tests including RocksDB
cargo test --workspace -p rucksfs-storage --features rocksdb
```

---

## Troubleshooting

### RocksDB compilation fails

**Symptom**: Errors about missing `libclang`, `cmake`, or C++ compiler.

**Fix**: Install build tools:
```bash
# Debian/Ubuntu
sudo apt-get install cmake clang libclang-dev

# macOS
brew install cmake llvm

# Fedora
sudo dnf install cmake clang clang-devel
```

### FUSE is not available

**Symptom**: `Error: not implemented` when using `--mount`.

**Fix**: Ensure you're on Linux with FUSE installed:
```bash
sudo apt-get install libfuse-dev fuse
sudo modprobe fuse
```

### Permission denied on mount

**Symptom**: `Permission denied` when mounting FUSE.

**Fix**: Ensure your user is in the `fuse` group, or run with `sudo`:
```bash
sudo usermod -a -G fuse $USER
# Log out and back in, then retry
```

### Data not persisting

**Symptom**: Files disappear after restart.

**Fix**: Make sure you're using `--persist` with the `rocksdb` feature:
```bash
cargo run -p rucksfs-demo --features rocksdb -- --persist /path/to/data
```

Without `--features rocksdb`, the `--persist` flag will print an error and exit.

---

## Architecture Overview

```
┌────────────────────────────────────┐
│           rucksfs-demo             │
│  (CLI: auto / interactive / FUSE)  │
├────────────┬───────────────────────┤
│ rucksfs-   │    rucksfs-server     │
│ client     │  (MetadataServer:     │
│            │   PosixOps+ClientOps) │
├────────────┴───────────────────────┤
│         rucksfs-storage            │
│  ┌────────────┐  ┌──────────────┐  │
│  │ Memory*    │  │ RocksDB*     │  │
│  │ (default)  │  │ (--persist)  │  │
│  └────────────┘  └──────────────┘  │
├────────────────────────────────────┤
│           rucksfs-core             │
│  (ClientOps, PosixOps, types)      │
└────────────────────────────────────┘
```
