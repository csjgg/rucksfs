//! RucksFS Demo — demonstrates the full file-system stack in a single process.
//!
//! Modes:
//!   (default)         Run an automatic demo showcasing all POSIX operations.
//!   --interactive     Enter an interactive REPL shell.
//!   --mount <path>    Mount as a FUSE filesystem (Linux only).
//!   --persist <dir>   Use RocksDB + RawDisk backends for persistent storage.

use std::sync::Arc;

use clap::Parser;
use rucksfs_client::build_client;
use rucksfs_core::ClientOps;
use rucksfs_server::MetadataServer;
use rucksfs_storage::{MemoryDataStore, MemoryDirectoryIndex, MemoryMetadataStore};
#[cfg(feature = "rocksdb")]
use rucksfs_storage::{open_rocks_db, RocksDirectoryIndex, RocksMetadataStore};
#[cfg(feature = "rocksdb")]
use rucksfs_storage::RawDiskDataStore;

/// RucksFS Demo — a single-binary demonstration of the full file-system stack.
#[derive(Parser, Debug)]
#[command(name = "rucksfs-demo", version, about)]
struct Cli {
    /// Enter interactive REPL mode instead of running the automatic demo.
    #[arg(long)]
    interactive: bool,

    /// Mount as a FUSE filesystem at the given path (Linux only).
    #[arg(long, value_name = "MOUNTPOINT")]
    mount: Option<String>,

    /// Use persistent storage (RocksDB + RawDisk) rooted at the given directory.
    /// Requires the `rocksdb` feature.
    #[arg(long, value_name = "DIR")]
    persist: Option<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Decide which mode to run based on CLI flags.
    // Priority: --mount > --interactive > auto-demo
    if let Some(ref _mountpoint) = cli.mount {
        run_mount_mode(&cli).await;
    } else if cli.interactive {
        run_interactive_mode(&cli).await;
    } else {
        run_auto_demo_mode(&cli).await;
    }
}

// ---------------------------------------------------------------------------
// Auto-demo mode
// ---------------------------------------------------------------------------

/// Run the automatic demo, exercising all major POSIX operations.
async fn run_auto_demo_mode(cli: &Cli) {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║         RucksFS — Automatic Demo                    ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    if let Some(ref dir) = cli.persist {
        #[cfg(feature = "rocksdb")]
        {
            println!("▶ Using persistent storage at: {}", dir);
            let db_path = std::path::Path::new(dir).join("metadata.db");
            let data_path = std::path::Path::new(dir).join("data.raw");

            // Ensure the directory exists
            std::fs::create_dir_all(dir).expect("failed to create persist directory");

            let db = open_rocks_db(&db_path).expect("failed to open RocksDB");
            let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
            let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
            let data = Arc::new(
                RawDiskDataStore::open(&data_path, 64 * 1024 * 1024)
                    .expect("failed to open RawDisk data store"),
            );

            let server = Arc::new(MetadataServer::new(metadata, data, index));
            let client = build_client(server);
            run_auto_demo(&client).await;
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = dir;
            eprintln!("Error: --persist requires the `rocksdb` feature.");
            eprintln!("Rebuild with: cargo run -p rucksfs-demo --features rocksdb -- --persist {}", dir);
            std::process::exit(1);
        }
    } else {
        println!("▶ Using in-memory storage (data will not survive restart)");
        let metadata = Arc::new(MemoryMetadataStore::new());
        let index = Arc::new(MemoryDirectoryIndex::new());
        let data = Arc::new(MemoryDataStore::new());
        let server = Arc::new(MetadataServer::new(metadata, data, index));
        let client = build_client(server);
        run_auto_demo(&client).await;
    }

    println!();
    println!("✔ Demo completed successfully!");
}

/// Core auto-demo logic: exercises all major operations sequentially.
async fn run_auto_demo(client: &impl ClientOps) {
    const ROOT: u64 = 1;
    println!();

    // 1. mkdir /mydir
    print_step(1, "mkdir /mydir");
    match client.mkdir(ROOT, "mydir", 0o755).await {
        Ok(attr) => println!("   ✓ Created directory inode={}, mode={:#o}", attr.inode, attr.mode),
        Err(e) => println!("   ✗ {}", e),
    }

    // 2. create /mydir/hello.txt
    print_step(2, "create /mydir/hello.txt");
    let mydir_inode = match client.lookup(ROOT, "mydir").await {
        Ok(attr) => attr.inode,
        Err(e) => {
            println!("   ✗ lookup /mydir failed: {}", e);
            return;
        }
    };
    let file_inode = match client.create(mydir_inode, "hello.txt", 0o644).await {
        Ok(attr) => {
            println!("   ✓ Created file inode={}, mode={:#o}", attr.inode, attr.mode);
            attr.inode
        }
        Err(e) => {
            println!("   ✗ {}", e);
            return;
        }
    };

    // 3. write "Hello, RucksFS!\n" to /mydir/hello.txt
    print_step(3, "write \"Hello, RucksFS!\" → /mydir/hello.txt");
    let content = b"Hello, RucksFS!\n";
    match client.write(file_inode, 0, content, 0).await {
        Ok(n) => println!("   ✓ Wrote {} bytes", n),
        Err(e) => println!("   ✗ {}", e),
    }

    // 4. read /mydir/hello.txt
    print_step(4, "read /mydir/hello.txt");
    match client.read(file_inode, 0, 4096).await {
        Ok(data) => {
            let text = String::from_utf8_lossy(&data);
            let trimmed = text.trim_end_matches('\0');
            println!("   ✓ Read {} bytes: {:?}", data.len(), trimmed);
        }
        Err(e) => println!("   ✗ {}", e),
    }

    // 5. readdir /mydir
    print_step(5, "readdir /mydir");
    match client.readdir(mydir_inode).await {
        Ok(entries) => {
            println!("   ✓ {} entries:", entries.len());
            for entry in &entries {
                println!("     - {} (inode={})", entry.name, entry.inode);
            }
        }
        Err(e) => println!("   ✗ {}", e),
    }

    // 6. rename /mydir/hello.txt → /mydir/greeting.txt
    print_step(6, "rename /mydir/hello.txt → /mydir/greeting.txt");
    match client.rename(mydir_inode, "hello.txt", mydir_inode, "greeting.txt").await {
        Ok(()) => println!("   ✓ Renamed successfully"),
        Err(e) => println!("   ✗ {}", e),
    }

    // 7. getattr /mydir/greeting.txt
    print_step(7, "getattr /mydir/greeting.txt");
    match client.lookup(mydir_inode, "greeting.txt").await {
        Ok(attr) => {
            println!("   ✓ inode={}, size={}, mode={:#o}, nlink={}", attr.inode, attr.size, attr.mode, attr.nlink);
        }
        Err(e) => println!("   ✗ {}", e),
    }

    // 8. unlink /mydir/greeting.txt
    print_step(8, "unlink /mydir/greeting.txt");
    match client.unlink(mydir_inode, "greeting.txt").await {
        Ok(()) => println!("   ✓ Unlinked successfully"),
        Err(e) => println!("   ✗ {}", e),
    }

    // 9. rmdir /mydir
    print_step(9, "rmdir /mydir");
    match client.rmdir(ROOT, "mydir").await {
        Ok(()) => println!("   ✓ Removed directory"),
        Err(e) => println!("   ✗ {}", e),
    }

    // 10. statfs
    print_step(10, "statfs /");
    match client.statfs(ROOT).await {
        Ok(st) => {
            println!("   ✓ blocks={}, bfree={}, files={}, bsize={}", st.blocks, st.bfree, st.files, st.bsize);
        }
        Err(e) => println!("   ✗ {}", e),
    }
}

/// Pretty-print a step header.
fn print_step(n: u32, desc: &str) {
    println!("── Step {:>2}: {} ──", n, desc);
}

// ---------------------------------------------------------------------------
// Interactive REPL mode
// ---------------------------------------------------------------------------

/// Build a client from CLI options and run the interactive REPL.
async fn run_interactive_mode(cli: &Cli) {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║         RucksFS — Interactive REPL                  ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    if let Some(ref dir) = cli.persist {
        #[cfg(feature = "rocksdb")]
        {
            println!("▶ Using persistent storage at: {}", dir);
            let db_path = std::path::Path::new(dir).join("metadata.db");
            let data_path = std::path::Path::new(dir).join("data.raw");
            std::fs::create_dir_all(dir).expect("failed to create persist directory");

            let db = open_rocks_db(&db_path).expect("failed to open RocksDB");
            let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
            let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
            let data = Arc::new(
                RawDiskDataStore::open(&data_path, 64 * 1024 * 1024)
                    .expect("failed to open RawDisk data store"),
            );
            let server = Arc::new(MetadataServer::new(metadata, data, index));
            let client = build_client(server);
            run_repl(&client).await;
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = dir;
            eprintln!("Error: --persist requires the `rocksdb` feature.");
            std::process::exit(1);
        }
    } else {
        println!("▶ Using in-memory storage (data will not survive restart)");
        let metadata = Arc::new(MemoryMetadataStore::new());
        let index = Arc::new(MemoryDirectoryIndex::new());
        let data = Arc::new(MemoryDataStore::new());
        let server = Arc::new(MetadataServer::new(metadata, data, index));
        let client = build_client(server);
        run_repl(&client).await;
    }
}

/// Root inode constant.
const ROOT_INODE: u64 = 1;

/// Resolve a POSIX-style path (e.g. "/foo/bar") to its inode by walking
/// the directory tree from root.  Returns `(parent_inode, last_component, target_inode)`.
async fn resolve_path(
    client: &impl ClientOps,
    path: &str,
) -> Result<(u64, String, u64), String> {
    let path = path.trim();
    if path == "/" {
        return Ok((ROOT_INODE, "/".to_string(), ROOT_INODE));
    }

    let path = path.strip_prefix('/').unwrap_or(path);
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if components.is_empty() {
        return Ok((ROOT_INODE, "/".to_string(), ROOT_INODE));
    }

    let mut current = ROOT_INODE;
    for (i, comp) in components.iter().enumerate() {
        if i == components.len() - 1 {
            // Last component — try to resolve but return parent info regardless
            match client.lookup(current, comp).await {
                Ok(attr) => return Ok((current, comp.to_string(), attr.inode)),
                Err(_) => return Err(format!("'{}' not found in path '/{}'", comp,
                    components[..=i].join("/"))),
            }
        } else {
            match client.lookup(current, comp).await {
                Ok(attr) => current = attr.inode,
                Err(_) => return Err(format!("directory '{}' not found in path '/{}'", comp,
                    components[..=i].join("/"))),
            }
        }
    }

    unreachable!()
}

/// Resolve a path to (parent_inode, child_name) — useful for operations
/// that need the parent context (create, mkdir, unlink, etc.).
async fn resolve_parent(
    client: &impl ClientOps,
    path: &str,
) -> Result<(u64, String), String> {
    let path = path.trim();
    let path = path.strip_prefix('/').unwrap_or(path);
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if components.is_empty() {
        return Err("cannot resolve parent of root".to_string());
    }

    let name = components.last().unwrap().to_string();

    if components.len() == 1 {
        return Ok((ROOT_INODE, name));
    }

    let mut current = ROOT_INODE;
    for comp in &components[..components.len() - 1] {
        match client.lookup(current, comp).await {
            Ok(attr) => current = attr.inode,
            Err(_) => return Err(format!("directory '{}' not found", comp)),
        }
    }

    Ok((current, name))
}

/// The interactive REPL loop.
async fn run_repl(client: &impl ClientOps) {
    use std::io::{self, BufRead, Write};

    println!("Type 'help' for available commands, 'exit' to quit.\n");

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        print!("rucksfs> ");
        io::stdout().flush().ok();

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Err(e) => {
                eprintln!("read error: {}", e);
                break;
            }
            _ => {}
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        let cmd = parts[0];

        match cmd {
            "help" => print_help(),

            "exit" | "quit" => {
                println!("Goodbye!");
                break;
            }

            "ls" => {
                let path = parts.get(1).copied().unwrap_or("/");
                match resolve_path(client, path).await {
                    Ok((_, _, inode)) => match client.readdir(inode).await {
                        Ok(entries) => {
                            if entries.is_empty() {
                                println!("  (empty directory)");
                            }
                            for e in &entries {
                                let kind = if e.kind & 0o040000 != 0 { "d" } else { "-" };
                                println!("  {} {} (inode={})", kind, e.name, e.inode);
                            }
                        }
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "mkdir" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: mkdir <path>"); continue; }
                };
                match resolve_parent(client, path).await {
                    Ok((parent, name)) => match client.mkdir(parent, &name, 0o755).await {
                        Ok(attr) => println!("  created directory inode={}", attr.inode),
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "touch" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: touch <path>"); continue; }
                };
                match resolve_parent(client, path).await {
                    Ok((parent, name)) => match client.create(parent, &name, 0o644).await {
                        Ok(attr) => println!("  created file inode={}", attr.inode),
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "write" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: write <path> <content>"); continue; }
                };
                let content = match parts.get(2) {
                    Some(c) => *c,
                    None => { eprintln!("  usage: write <path> <content>"); continue; }
                };
                match resolve_path(client, path).await {
                    Ok((_, _, inode)) => {
                        let data = content.as_bytes();
                        match client.write(inode, 0, data, 0).await {
                            Ok(n) => println!("  wrote {} bytes", n),
                            Err(e) => eprintln!("  error: {}", e),
                        }
                    }
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "cat" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: cat <path>"); continue; }
                };
                match resolve_path(client, path).await {
                    Ok((_, _, inode)) => match client.read(inode, 0, 1024 * 1024).await {
                        Ok(data) => {
                            let text = String::from_utf8_lossy(&data);
                            let trimmed = text.trim_end_matches('\0');
                            if trimmed.is_empty() {
                                println!("  (empty file)");
                            } else {
                                println!("{}", trimmed);
                            }
                        }
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "rm" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: rm <path>"); continue; }
                };
                match resolve_parent(client, path).await {
                    Ok((parent, name)) => match client.unlink(parent, &name).await {
                        Ok(()) => println!("  removed"),
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "rmdir" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: rmdir <path>"); continue; }
                };
                match resolve_parent(client, path).await {
                    Ok((parent, name)) => match client.rmdir(parent, &name).await {
                        Ok(()) => println!("  removed directory"),
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "mv" => {
                let src = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: mv <src> <dst>"); continue; }
                };
                let dst = match parts.get(2) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: mv <src> <dst>"); continue; }
                };
                let src_parent = match resolve_parent(client, src).await {
                    Ok(v) => v,
                    Err(e) => { eprintln!("  error resolving src: {}", e); continue; }
                };
                let dst_parent = match resolve_parent(client, dst).await {
                    Ok(v) => v,
                    Err(e) => { eprintln!("  error resolving dst: {}", e); continue; }
                };
                match client.rename(src_parent.0, &src_parent.1, dst_parent.0, &dst_parent.1).await {
                    Ok(()) => println!("  renamed"),
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "stat" => {
                let path = match parts.get(1) {
                    Some(p) => *p,
                    None => { eprintln!("  usage: stat <path>"); continue; }
                };
                match resolve_path(client, path).await {
                    Ok((_, _, inode)) => match client.getattr(inode).await {
                        Ok(attr) => {
                            let kind = if attr.mode & 0o040000 != 0 { "directory" } else { "file" };
                            println!("  inode:  {}", attr.inode);
                            println!("  type:   {}", kind);
                            println!("  size:   {} bytes", attr.size);
                            println!("  mode:   {:#o}", attr.mode);
                            println!("  nlink:  {}", attr.nlink);
                            println!("  uid:    {}", attr.uid);
                            println!("  gid:    {}", attr.gid);
                        }
                        Err(e) => eprintln!("  error: {}", e),
                    },
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            "statfs" => {
                match client.statfs(ROOT_INODE).await {
                    Ok(st) => {
                        println!("  blocks:  {}", st.blocks);
                        println!("  bfree:   {}", st.bfree);
                        println!("  bavail:  {}", st.bavail);
                        println!("  files:   {}", st.files);
                        println!("  ffree:   {}", st.ffree);
                        println!("  bsize:   {}", st.bsize);
                        println!("  namelen: {}", st.namelen);
                    }
                    Err(e) => eprintln!("  error: {}", e),
                }
            }

            _ => {
                eprintln!("  unknown command: '{}'. Type 'help' for usage.", cmd);
            }
        }
    }
}

/// Print the help text listing all REPL commands.
fn print_help() {
    println!("Available commands:");
    println!("  ls [path]              List directory contents (default: /)");
    println!("  mkdir <path>           Create a directory");
    println!("  touch <path>           Create an empty file");
    println!("  write <path> <text>    Write text content to a file");
    println!("  cat <path>             Read and display file content");
    println!("  rm <path>              Remove a file");
    println!("  rmdir <path>           Remove an empty directory");
    println!("  mv <src> <dst>         Rename / move a file or directory");
    println!("  stat <path>            Show file or directory attributes");
    println!("  statfs                 Show filesystem statistics");
    println!("  help                   Show this help message");
    println!("  exit                   Exit the REPL");
}

// ---------------------------------------------------------------------------
// FUSE mount mode
// ---------------------------------------------------------------------------

async fn run_mount_mode(cli: &Cli) {
    let mountpoint = cli.mount.as_deref().unwrap_or("/tmp/rucksfs");

    #[cfg(target_os = "linux")]
    {
        println!("╔══════════════════════════════════════════════════════╗");
        println!("║         RucksFS — FUSE Mount Mode                   ║");
        println!("╚══════════════════════════════════════════════════════╝");
        println!();
        println!("▶ Mounting at: {}", mountpoint);

        // Ensure the mountpoint directory exists
        if let Err(e) = std::fs::create_dir_all(mountpoint) {
            eprintln!("Error: failed to create mountpoint '{}': {}", mountpoint, e);
            std::process::exit(1);
        }

        if let Some(ref dir) = cli.persist {
            #[cfg(feature = "rocksdb")]
            {
                println!("▶ Using persistent storage at: {}", dir);
                std::fs::create_dir_all(dir).expect("failed to create persist directory");

                let db_path = std::path::Path::new(dir).join("metadata.db");
                let data_path = std::path::Path::new(dir).join("data.raw");

                let db = open_rocks_db(&db_path).expect("failed to open RocksDB");
                let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
                let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
                let data = Arc::new(
                    RawDiskDataStore::open(&data_path, 64 * 1024 * 1024)
                        .expect("failed to open RawDisk data store"),
                );
                let server = Arc::new(MetadataServer::new(metadata, data, index));
                let client = Arc::new(build_client(server));

                println!("▶ Press Ctrl+C or run `fusermount -u {}` to unmount.", mountpoint);
                if let Err(e) = rucksfs_client::mount_fuse(mountpoint, client) {
                    eprintln!("FUSE mount error: {}", e);
                    std::process::exit(1);
                }
            }
            #[cfg(not(feature = "rocksdb"))]
            {
                let _ = dir;
                eprintln!("Error: --persist requires the `rocksdb` feature.");
                std::process::exit(1);
            }
        } else {
            println!("▶ Using in-memory storage");
            let metadata = Arc::new(MemoryMetadataStore::new());
            let index = Arc::new(MemoryDirectoryIndex::new());
            let data = Arc::new(MemoryDataStore::new());
            let server = Arc::new(MetadataServer::new(metadata, data, index));
            let client = Arc::new(build_client(server));

            println!("▶ Press Ctrl+C or run `fusermount -u {}` to unmount.", mountpoint);
            if let Err(e) = rucksfs_client::mount_fuse(mountpoint, client) {
                eprintln!("FUSE mount error: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mountpoint;
        eprintln!("⚠ FUSE mount is only supported on Linux.");
        eprintln!("  Falling back to auto-demo...");
        println!();
        run_auto_demo_mode(cli).await;
    }
}
