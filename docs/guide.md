# RucksFS Usage Guide

RucksFS is a single-binary FUSE filesystem backed by RocksDB metadata and local file storage. This guide covers building, running, and testing.

---

## Prerequisites

| Requirement | Version | Notes |
|---|---|---|
| Rust toolchain | ≥ 1.70 | Install via [rustup](https://rustup.rs/) |
| RocksDB (optional) | — | Only needed for `--persist` mode; the `rocksdb` crate builds it from source automatically |
| FUSE / libfuse-dev (optional) | — | Only needed for `--mount` mode on Linux |

### Platform Notes

- **macOS / Windows**: Auto-demo and interactive modes work. FUSE mount is Linux-only.
- **Linux**: All three modes (auto-demo, interactive, FUSE mount) are available.

---

## Quick Start

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

```bash
cargo run -p rucksfs-demo
```

Executes 10 steps:

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
To persist data across restarts:

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

### FUSE Mount Options

RucksFS automatically sets the following FUSE mount options:

| Option | Purpose |
|---|---|
| `fsname=rucksfs` | Identifies the filesystem in `/proc/mounts` |
| `auto_unmount` | Automatically unmounts on process exit |
| `default_permissions` | **Delegates POSIX permission checks to the Linux kernel VFS layer.** The kernel enforces `rwx` bits, ownership (`uid`/`gid`), and open flags before requests reach the FUSE daemon. |
| `allow_other` | Allows users other than the mounter to access the filesystem |

#### Enabling `allow_other`

The `allow_other` option requires configuration in `/etc/fuse.conf`:

```bash
# Edit /etc/fuse.conf and uncomment or add:
user_allow_other
```

Without this setting, mounting will fail with a permission error. If you are running as root, this restriction does not apply.

#### How Permission Enforcement Works

With `default_permissions` enabled:

1. When a user calls `open()`, `mkdir()`, `unlink()`, etc., the **kernel** checks the inode's `uid`/`gid`/`mode` against the caller's credentials.
2. If the check fails, the kernel returns `EACCES` immediately — the request **never reaches** the RucksFS FUSE daemon.
3. RucksFS correctly sets `uid`/`gid` from the calling process on `create` and `mkdir`, and applies the user's `umask` to the file mode.

This provides standard POSIX permission semantics (owner/group/other, `rwx` bits) with zero overhead in the FUSE daemon.

---

## TLS & Authentication

For distributed or security-sensitive deployments, RucksFS supports TLS encryption and Bearer token authentication via the gRPC layer.

### Generating TLS Certificates

#### Self-Signed (Development)

```bash
# Generate CA
openssl genrsa -out ca.key 4096
openssl req -new -x509 -days 365 -key ca.key -out ca.crt -subj "/CN=RucksFS CA"

# Generate server cert
openssl genrsa -out server.key 4096
openssl req -new -key server.key -out server.csr -subj "/CN=localhost"
openssl x509 -req -days 365 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt
```

#### Production

Use certificates from a trusted CA (e.g., Let's Encrypt):

```bash
certbot certonly --standalone -d your-server.example.com
```

### Security Best Practices

1. **Always use TLS in production** — prevents eavesdropping and MITM attacks.
2. **Use strong tokens** — generate with `openssl rand -hex 32`.
3. **Restrict network access** — firewall rules, bind to specific interfaces.
4. **Secure certificate storage** — `chmod 600 server.key`, rotate before expiration.
5. **Use environment variables for secrets**:
   ```bash
   export RUCKSFS_TOKEN="your-secret-token"
   ```

---

## Testing

### Unit & Integration Tests

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

### VfsOps Stress Tests (Any OS)

In-process concurrency tests via `EmbeddedClient` + `tokio::spawn`:

```bash
cargo test -p rucksfs-demo --test stress_test
cargo test -p rucksfs-demo --test stress_test -- --nocapture
```

**What they verify:**

- Concurrent file creation (100+ files in parallel)
- Concurrent mkdir in the same parent
- Race condition on same-name creation (exactly 1 winner)
- Concurrent writes at different offsets (data integrity)
- Mixed concurrent read + write
- Concurrent readdir + mkdir (consistent snapshots)
- Concurrent rename (no ghost / lost files)
- Concurrent unlink + lookup (no panics)
- Metadata consistency after mass mutations
- Large-scale creation (500+ files)
- Concurrent setattr
- Cross-directory concurrent rename
- Create + write + read + unlink storm
- Deep nested concurrent operations
- Concurrent statfs

### FUSE E2E Tests (Linux Only)

#### Built-in E2E Script

```bash
# Basic run with in-memory storage
./scripts/e2e_fuse_test.sh

# With persistent RocksDB storage
./scripts/e2e_fuse_test.sh --persist /tmp/rucksfs_data

# Custom mount point
./scripts/e2e_fuse_test.sh --mountpoint /mnt/rucksfs

# Combined
./scripts/e2e_fuse_test.sh --persist /tmp/rucksfs_data --mountpoint /mnt/rucksfs
```

The script builds the project, mounts via FUSE, runs basic operations + write-pattern tests + concurrent stress tests + metadata consistency checks, then unmounts and reports results.

#### pjdfstest (POSIX Compliance)

[pjdfstest](https://github.com/pjd/pjdfstest) is an industry-standard POSIX filesystem compliance test suite.

```bash
# Install
git clone https://github.com/pjd/pjdfstest.git && cd pjdfstest
autoreconf -ifs && ./configure && make

# Mount RucksFS
./target/debug/rucksfs-demo --mount /tmp/rucksfs_e2e &

# Run tests
cd /tmp/rucksfs_e2e
sudo prove -r /path/to/pjdfstest/tests/

# Unmount
fusermount -u /tmp/rucksfs_e2e
```

**Expected test status:**

| Category | Priority | Status |
|----------|----------|--------|
| `mkdir` | High | Should pass |
| `rmdir` | High | Should pass |
| `open` / `create` | High | Should pass |
| `unlink` | High | Should pass |
| `rename` | High | Should pass |
| `truncate` | Medium | Should pass |
| `chmod` | Medium | Should pass (via setattr) |
| `chown` | Low | Partial support |
| `link` (hard link) | Low | Not yet implemented |
| `symlink` | Low | Not yet implemented |
| `mkfifo` | Low | Not applicable |

### Recommended Testing Workflow

```
1. Development (macOS / any OS)
   └─> cargo test -p rucksfs-demo --test stress_test

2. Pre-release (Linux)
   └─> ./scripts/e2e_fuse_test.sh --persist /tmp/rucksfs_data
   └─> Run pjdfstest for POSIX compliance

3. CI (Linux with FUSE)
   └─> cargo test (all unit + integration + stress tests)
   └─> ./scripts/e2e_fuse_test.sh
```

---

## Architecture Overview

```
┌──────────────────────────────────────────────┐
│               rucksfs-demo                   │
│  (CLI: auto-demo / interactive / FUSE)       │
├──────────────────────────────────────────────┤
│             rucksfs-client                   │
│  ┌────────────────────────────────────────┐  │
│  │ EmbeddedClient (in-process)            │  │
│  │ VfsCore (routing: metadata ↔ data)     │  │
│  └────────────┬───────────┬───────────────┘  │
│         MetadataOps     DataOps              │
├──────────────┬────────────┬──────────────────┤
│ rucksfs-     │            │ rucksfs-         │
│ server       │            │ dataserver       │
│ (MetadataServer)          │ (DataServer)     │
├──────────────┴────────────┴──────────────────┤
│            rucksfs-storage                   │
│  ┌──────────────┐  ┌─────────────────────┐   │
│  │ Memory*      │  │ RocksDB + RawDisk*  │   │
│  │ (default)    │  │ (--persist)         │   │
│  └──────────────┘  └─────────────────────┘   │
├──────────────────────────────────────────────┤
│              rucksfs-core                    │
│  (MetadataOps, DataOps, VfsOps, types)       │
└──────────────────────────────────────────────┘
```

---

## Troubleshooting

### RocksDB compilation fails

Install build tools:
```bash
# Debian/Ubuntu
sudo apt-get install cmake clang libclang-dev

# macOS
brew install cmake llvm

# Fedora
sudo dnf install cmake clang clang-devel
```

### FUSE is not available

Ensure you're on Linux with FUSE installed:
```bash
sudo apt-get install libfuse-dev fuse
sudo modprobe fuse
```

### Permission denied on mount

Ensure your user is in the `fuse` group:
```bash
sudo usermod -a -G fuse $USER
# Log out and back in, then retry
```

### Data not persisting

Make sure you're using `--persist` with the `rocksdb` feature:
```bash
cargo run -p rucksfs-demo --features rocksdb -- --persist /path/to/data
```

Without `--features rocksdb`, the `--persist` flag will print an error and exit.
