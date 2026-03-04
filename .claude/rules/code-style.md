# Rust Code Style for RucksFS

## Language

- Code comments: English
- Conversation with user: Chinese
- Documentation (guide.md, README): English
- Commit messages: English

## Naming Conventions

- Types: `PascalCase` (e.g., `MetadataServer`, `InodeValue`)
- Functions/methods: `snake_case` (e.g., `insert_child`, `report_write`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `ROOT_INODE`, `DELTA_COMPACT_THRESHOLD`)
- Crate names: `rucksfs-*` with hyphens (e.g., `rucksfs-server`, `rucksfs-storage`)
- Module names: `snake_case` (e.g., `delta.rs`, `compaction.rs`)

## Error Handling

- Use `std::io::Result<T>` for all trait methods (consistent with FUSE/POSIX conventions)
- Convert internal errors to `io::Error` with appropriate `ErrorKind`
- Common mappings:
  - Not found -> `ErrorKind::NotFound` (ENOENT)
  - Already exists -> `ErrorKind::AlreadyExists` (EEXIST)
  - Not a directory -> custom `from_raw_os_error(libc::ENOTDIR)`
  - Is a directory -> custom `from_raw_os_error(libc::EISDIR)`
  - Directory not empty -> custom `from_raw_os_error(libc::ENOTEMPTY)`
  - Transaction conflict -> custom `from_raw_os_error(libc::EAGAIN)`

## Trait Design

- Define traits in `core/src/lib.rs`
- Use `&self` receivers (implementations hold internal `Arc<Mutex<...>>` if needed)
- Return `std::io::Result<T>`
- Keep trait methods focused — one operation per method

## Testing

- Use `tempfile::tempdir()` for test isolation
- Prefer RocksDB-backed tests (no memory mocks)
- Test names: `test_<operation>_<scenario>` (e.g., `test_create_duplicate_returns_eexist`)
- Use `#[should_panic]` sparingly; prefer asserting `Result::Err`

## Imports

- Group imports: std -> external crates -> internal crates
- Use explicit imports, avoid glob `use module::*`
- Exception: `use std::io::Result` is acceptable as a convenience alias

## Formatting

- Follow `rustfmt` defaults
- Max line width: 100 characters (soft limit)
- Use trailing commas in multi-line argument lists
