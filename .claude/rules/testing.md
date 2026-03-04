# Testing Guidelines for RucksFS

## Test Hierarchy

```
1. Unit tests       -> cargo test --workspace          (fast, ~192 tests)
2. Local FUSE E2E   -> ./scripts/e2e_fuse_test.sh      (needs Linux + FUSE)
3. Remote E2E       -> task remote-test                 (needs SSH config)
4. Benchmark only   -> task remote-test:bench-only      (needs deployed instance)
5. Quick verify     -> task remote-test:quick            (correctness only)
```

## When to Run What

| Scenario | Command |
|----------|---------|
| After any code change | `cargo test --workspace` |
| After modifying a single crate | `cargo test -p rucksfs-server` (or specific crate) |
| After changing FUSE behavior | `cargo test --workspace` + `./scripts/e2e_fuse_test.sh` |
| Before submitting a feature | Full remote E2E: `task remote-test` |
| Performance tuning iteration | `task remote-test:bench-only` |
| Quick regression check | `task remote-test:quick` |

## Writing New Tests

### Unit/Integration Tests

- Place in `demo/tests/integration_test.rs` for full-stack tests
- Place in `server/tests/` for metadata-only tests
- Always use `tempfile::tempdir()` for isolation
- Use real RocksDB backends, not mocks
- Clean up after tests (tempdir handles this automatically)

### Test Structure

```rust
#[test]
fn test_operation_scenario() {
    // Setup: create temp dir, init storage, create server
    let dir = tempfile::tempdir().unwrap();
    // ... setup code ...

    // Act: perform the operation
    let result = server.create(parent, "filename", mode, uid, gid);

    // Assert: verify the result
    assert!(result.is_ok());
    let attr = result.unwrap();
    assert_eq!(attr.size, 0);
}
```

### Naming Convention

- `test_<operation>_<scenario>` for positive cases
- `test_<operation>_<error_condition>` for negative cases
- Examples:
  - `test_create_new_file_returns_correct_attrs`
  - `test_create_duplicate_returns_eexist`
  - `test_rmdir_notempty_returns_error`

## Remote E2E Test Suites

The remote testing pipeline includes three suites:

1. **pjdfstest** — POSIX compliance (known limitations: link, symlink, mkfifo, mknod)
2. **Correctness tests** — create+stat, write+read, rename atomicity, etc.
3. **Metadata benchmark** — create/stat/unlink/mkdir/readdir throughput + latency

Results are saved to `test-results/<timestamp>/report.json` (structured) and `report.md` (human-readable).

## Known Test Environment Requirements

- RocksDB: always available (unconditional dependency)
- FUSE: Linux only, requires `/dev/fuse` and `fuser` crate
- `/etc/fuse.conf`: must have `user_allow_other` for `AllowOther` mount option
- Remote tests: requires SSH access configured in `scripts/remote-test/test-config.toml`
- `protoc`: needed only for `rpc` crate (excluded from default workspace build)
