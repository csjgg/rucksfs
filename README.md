# rucksfs

RucksFS: 元数据管理（FUSE + RocksDB）工程骨架。

## Workspace

- `core`：公共类型、POSIX 接口定义
- `storage`：元数据/数据存储抽象
- `server`：Metadata Server（同步 POSIX 核心）
- `client`：FUSE Client + Client 抽象
- `rpc`：RPC 预留
- `demo`：单进程 demo（二进制）

## Build

```bash
cargo check
cargo build -p rucksfs-demo
```

## Demo

默认不挂载 FUSE。如需挂载（需要系统权限）：

```bash
cargo run -p rucksfs-demo -- --mount
```
