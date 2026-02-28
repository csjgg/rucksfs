# Deployment Guide

> **⚠️ Note:** This guide was written for the earlier single-server architecture.
> RucksFS has since been refactored into a split **MetadataServer + DataServer** design.
> The TLS/authentication concepts below still apply, but the CLI examples reference
> a single `rucksfs-server` / `rucksfs-client` binary that has not yet been updated
> to the new two-service model. A revised deployment guide will be published once the
> standalone `RucksClient` (gRPC network client) and separate server binaries are implemented.
> See the project [README](../README.md) TODO section for current status.

This guide explains how to configure authentication and TLS for secure communication between the RucksFS server and client.

## Overview

The RucksFS RPC layer uses gRPC over TLS for secure communication. In the new architecture, two services need to be deployed:

- **MetadataServer** — handles namespace, inodes, and directory entries (gRPC `MetadataService`)
- **DataServer** — handles file data I/O (gRPC `DataService`)

Both servers (and the client) can be configured with:

- **Authentication**: Bearer token-based authentication
- **Encryption**: TLS 1.3 for encrypted connections
- **Certificate Verification**: Optional CA certificate validation

## Generating TLS Certificates

### Self-Signed Certificates (Development)

For development or testing, you can generate self-signed certificates:

```bash
# Generate CA private key
openssl genrsa -out ca.key 4096

# Generate CA certificate
openssl req -new -x509 -days 365 -key ca.key -out ca.crt -subj "/CN=RucksFS CA"

# Generate server private key
openssl genrsa -out server.key 4096

# Generate server CSR
openssl req -new -key server.key -out server.csr -subj "/CN=localhost"

# Sign server certificate with CA
openssl x509 -req -days 365 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt
```

### Production Certificates

For production, use certificates from a trusted CA (e.g., Let's Encrypt):

```bash
certbot certonly --standalone -d your-server.example.com
```

## Server Configuration

### Basic Server (Insecure - Development Only)

```bash
rucksfs-server --bind 127.0.0.1:50051
```

### Server with Authentication

```bash
rucksfs-server --bind 127.0.0.1:50051 --token "your-secret-api-token"
```

### Server with TLS

```bash
rucksfs-server --bind 0.0.0.0:50051 \
    --tls-cert /path/to/server.crt \
    --tls-key /path/to/server.key
```

### Secure Server (Production Recommended)

```bash
rucksfs-server --bind 0.0.0.0:50051 \
    --token "your-secret-api-token" \
    --tls-cert /etc/rucksfs/server.crt \
    --tls-key /etc/rucksfs/server.key
```

### Server Options

| Option | Description | Required |
|--------|-------------|----------|
| `--bind <addr>` | Bind address (e.g., `0.0.0.0:50051`) | Yes |
| `--token <token>` | API token for Bearer authentication | Recommended |
| `--tls-cert <path>` | Path to TLS certificate file | Optional* |
| `--tls-key <path>` | Path to TLS private key file | Optional* |

* Both `--tls-cert` and `--tls-key` must be provided together.

## Client Configuration

### Basic Client (Insecure)

```bash
rucksfs-client --server http://127.0.0.1:50051
```

### Client with Authentication

```bash
rucksfs-client --server http://127.0.0.1:50051 \
    --token "your-secret-api-token"
```

### Client with TLS

```bash
rucksfs-client --server https://server.example.com:50051
```

### Client with TLS and Authentication (Production Recommended)

```bash
rucksfs-client --server https://server.example.com:50051 \
    --token "your-secret-api-token" \
    --ca-cert /etc/rucksfs/ca.crt \
    --mount /mnt/rucksfs
```

### Client Options

| Option | Description | Required |
|--------|-------------|----------|
| `--server <addr>` | Server address (e.g., `https://server:50051`) | Yes |
| `--token <token>` | API token for Bearer authentication | Recommended |
| `--ca-cert <path>` | Path to CA certificate for verification | Optional |
| `--domain <name>` | Server domain name for TLS verification | Optional |
| `--mount <path>` | Mount point (Linux only) | Optional |

## Security Best Practices

### 1. Always Use TLS in Production

TLS encrypts all traffic between client and server, preventing:

- Data interception and eavesdropping
- Man-in-the-middle attacks
- Credential leakage

### 2. Use Strong Authentication Tokens

Generate strong, random tokens:

```bash
# Generate a secure random token
openssl rand -hex 32
```

### 3. Restrict Network Access

- Use firewall rules to limit access to trusted IPs
- Bind to specific interfaces (`127.0.0.1` for local, `0.0.0.0` for remote)
- Consider using VPNs for additional security

### 4. Secure Certificate Storage

- Set proper file permissions: `chmod 600 server.key`
- Store private keys in secure directories
- Rotate certificates before expiration

### 5. Use Environment Variables for Secrets

```bash
# Server
export RUCKSFS_TOKEN="your-secret-token"
rucksfs-server --bind 0.0.0.0:50051 --token "$RUCKSFS_TOKEN"

# Client
rucksfs-client --server https://server:50051 --token "$RUCKSFS_TOKEN"
```

## Troubleshooting

### Connection Refused

- Check if server is running
- Verify bind address and port
- Check firewall rules

### TLS Handshake Failed

- Verify certificate paths
- Ensure certificate is not expired
- Check certificate chain validity

### Authentication Failed

- Verify token matches server token
- Check for typos in token
- Ensure token is passed correctly

### Certificate Verification Failed

- Ensure CA certificate is provided to client
- Check domain name matches certificate CN/SAN
- Verify certificate is signed by trusted CA

## Example Deployment

### Production Setup with Docker

```yaml
# docker-compose.yml
version: '3.8'
services:
  rucksfs-server:
    image: rucksfs-server:latest
    ports:
      - "50051:50051"
    environment:
      - RUCKSFS_TOKEN=${TOKEN}
    volumes:
      - ./certs:/etc/rucksfs/certs:ro
    command:
      - --bind
      - 0.0.0.0:50051
      - --token
      - ${TOKEN}
      - --tls-cert
      - /etc/rucksfs/certs/server.crt
      - --tls-key
      - /etc/rucksfs/certs/server.key
```

```bash
# Generate .env file
echo "TOKEN=$(openssl rand -hex 32)" > .env

# Start server
docker-compose up -d
```

## Port Configuration

The default gRPC port is `50051`. You can use any available port:

```bash
# Server on custom port
rucksfs-server --bind 0.0.0.0:9100 --token "$TOKEN" --tls-cert cert.crt --tls-key cert.key

# Client connecting to custom port
rucksfs-client --server https://server.example.com:9100 --token "$TOKEN"
```

## Next Steps

- Monitor server logs for authentication failures
- Set up log aggregation for security auditing
- Consider implementing rate limiting for production
- Regularly rotate authentication tokens and certificates

---

# E2E Testing Guide

This section describes how to run end-to-end tests against a live RucksFS FUSE mount to verify correctness, concurrency safety, and POSIX compliance.

## Prerequisites

- **Linux** with FUSE support (`fuse3` or `fuse` kernel module)
- Rust toolchain (for building the project)
- `fusermount` or `fusermount3` available in `$PATH`

## Testing Layers

RucksFS E2E testing is organized into two layers:

| Layer | Scope | Platform | Tool |
|-------|-------|----------|------|
| **VfsOps stress tests** | In-process concurrency via `EmbeddedClient` | Any (macOS, Linux) | `cargo test` |
| **FUSE E2E tests** | Real FUSE mount with POSIX operations | Linux only | Shell script + pjdfstest |

### Layer 1: VfsOps Stress Tests (Any OS)

These tests exercise the full MetadataServer + DataServer + EmbeddedClient stack
using in-memory storage with heavy concurrency via `tokio::spawn`.

```bash
# Run all stress / concurrency tests
cargo test -p rucksfs-demo --test stress_test

# Run with output visible
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

### Layer 2: FUSE E2E Tests (Linux Only)

#### Option A: Built-in E2E Script

The project includes an E2E shell script at `scripts/e2e_fuse_test.sh` that:

1. Builds the project
2. Mounts RucksFS via FUSE
3. Runs basic operations (mkdir, write, read, rename, unlink, rmdir)
4. Runs write-pattern tests (large files, append, data integrity via checksum)
5. Runs concurrent stress tests (parallel create, write, mkdir, create-delete storm)
6. Checks metadata consistency (stat sizes, chmod)
7. Unmounts and reports results

**Usage:**

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

**Example output:**

```
╔══════════════════════════════════════════════════════╗
║       RucksFS — E2E FUSE Test Suite                 ║
╚══════════════════════════════════════════════════════╝

── Building rucksfs-demo ──
Build OK

── Mounting FUSE at /tmp/rucksfs_e2e ──
FUSE mounted (PID=12345)

══ Test Suite 1: Basic File Operations ══
  ✓ PASS: mkdir creates directory
  ✓ PASS: create file
  ✓ PASS: write and read
  ...

══════════════════════════════════════════════════
Results: 20 passed, 0 failed, 20 total
══════════════════════════════════════════════════
```

#### Option B: pjdfstest (POSIX Compliance)

[pjdfstest](https://github.com/pjd/pjdfstest) is an industry-standard POSIX
filesystem compliance test suite. It covers `chmod`, `chown`, `link`, `mkdir`,
`mkfifo`, `open`, `rename`, `rmdir`, `symlink`, `truncate`, `unlink`, and more.

##### Installing pjdfstest

```bash
# Clone
git clone https://github.com/pjd/pjdfstest.git
cd pjdfstest

# Build
autoreconf -ifs
./configure
make
```

##### Running pjdfstest Against RucksFS

**Step 1**: Mount RucksFS via FUSE.

```bash
# Build with RocksDB if you want persistence
cargo build -p rucksfs-demo --features rocksdb

# Mount (in-memory)
./target/debug/rucksfs-demo --mount /tmp/rucksfs_e2e &

# Or mount with persistence
./target/debug/rucksfs-demo --mount /tmp/rucksfs_e2e --persist /tmp/rucksfs_data &
```

**Step 2**: Run pjdfstest.

```bash
cd /tmp/rucksfs_e2e

# Run all tests (as root — required for chown/chmod tests)
sudo prove -r /path/to/pjdfstest/tests/

# Run specific test categories
sudo prove /path/to/pjdfstest/tests/mkdir/
sudo prove /path/to/pjdfstest/tests/open/
sudo prove /path/to/pjdfstest/tests/rename/
sudo prove /path/to/pjdfstest/tests/unlink/
sudo prove /path/to/pjdfstest/tests/rmdir/
sudo prove /path/to/pjdfstest/tests/truncate/
```

> **Note:** Some pjdfstest tests require root privileges for `chown` and special
> file operations. Tests for `link` (hard links), `symlink`, and `mkfifo` will
> fail if RucksFS does not yet implement those operations — this is expected.

##### Interpreting pjdfstest Results

```
/path/to/pjdfstest/tests/mkdir/00.t .. ok
/path/to/pjdfstest/tests/mkdir/01.t .. ok
/path/to/pjdfstest/tests/open/00.t  .. ok
/path/to/pjdfstest/tests/rename/00.t .. ok
...
All tests successful.
Files=42, Tests=580, 12 wallclock secs
Result: PASS
```

Tests that fail indicate POSIX operations that need to be implemented or fixed.
Focus on the following categories first (which RucksFS already supports):

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

**Step 3**: Unmount.

```bash
fusermount -u /tmp/rucksfs_e2e
```

## Recommended Testing Workflow

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
