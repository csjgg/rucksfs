# Communication Layer Optimization — Work in Progress

**Goal**: Reduce the number of synchronous RPCs per file create from 5 to 2-3,
so that the 4.7× delta mechanism speedup (already measured in-process) can be
partially realized at end-to-end throughput.

## Measurement Baseline (before any optimization)

- mdtest shared parent, N=16: **770 ops/s** (FUSE + gRPC)
- in-process with-delta T=16: 349,349 ops/s
- Headroom: **450×**. Current bottleneck is entirely communication layer.

## Planned Changes (ordered by risk)

### Phase 1: Async release (lowest risk)
- File: `client/src/fuse.rs`
- Change: `release()` callback returns `Ok(())` immediately; spawns a background
  task to call `client.release(inode)`.  
- Risk: release can fail after FUSE already replied; mitigated by best-effort
  logging since POSIX doesn't require close() to report errors.

### Phase 2: Skip flush for untouched files
- Files: `client/src/fuse.rs`, `client/src/vfs_core.rs`
- Change: Track per-fh "has been written" bit; `flush()` on an unwritten fh
  returns Ok(()) without RPC.
- Risk: very low; matches Linux kernel behavior (flush is advisory).

### Phase 3: Merge create + open
- Files: `core/src/lib.rs`, `server/src/lib.rs`, `client/src/vfs_core.rs`,
  `client/src/fuse.rs`, `rpc/proto/metadata.proto`, `rpc/src/metadata_*.rs`
- Change: Add `CreateAndOpen` RPC; MetadataServer does both in single transaction;
  FUSE layer calls the merged op.
- Risk: moderate; requires proto change + backward-compat trait default.

## Expected Impact

| Phase | per-create sync RPCs | Expected end-to-end ops/s (shared dir, N=16) |
|-------|---------------------|---------------------------------------------|
| Baseline | 5 (lookup+create+open+flush+release) | 770 |
| +Async release | 4 | ~900 |
| +Skip flush | 3 | ~1,100 |
| +Merge create+open | 2 | ~1,500–2,000 |
| (Optional) negative lookup cache | 1 | ~2,500–3,000 |

## Status

- [ ] Phase 1: Async release
- [ ] Phase 2: Skip flush
- [ ] Phase 3: Merge create+open
- [ ] End-to-end benchmark comparison
