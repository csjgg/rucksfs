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
// Interactive mode (placeholder — implemented in task 6)
// ---------------------------------------------------------------------------

async fn run_interactive_mode(cli: &Cli) {
    println!("Interactive REPL mode — not yet implemented.");
    println!("Falling back to auto-demo...");
    run_auto_demo_mode(cli).await;
}

// ---------------------------------------------------------------------------
// FUSE mount mode (placeholder — implemented in task 7)
// ---------------------------------------------------------------------------

async fn run_mount_mode(cli: &Cli) {
    #[cfg(target_os = "linux")]
    {
        println!("FUSE mount mode — not yet implemented.");
        println!("Falling back to auto-demo...");
        run_auto_demo_mode(cli).await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("⚠ FUSE mount is only supported on Linux.");
        eprintln!("  Falling back to auto-demo...");
        println!();
        run_auto_demo_mode(cli).await;
    }
}
