pub mod client;
#[cfg(target_os = "linux")]
pub mod fuse;

pub use client::{build_client, Client, InProcessClient};
#[cfg(target_os = "linux")]
pub use fuse::fs_error_to_errno;
#[cfg(target_os = "linux")]
pub use fuse::mount_fuse;

#[cfg(target_os = "linux")]
pub use fuse::FuseClient;
