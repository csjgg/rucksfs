pub mod embedded;
pub mod vfs_core;
#[cfg(target_os = "linux")]
pub mod fuse;

pub use embedded::EmbeddedClient;
pub use vfs_core::VfsCore;

#[cfg(target_os = "linux")]
pub use fuse::fs_error_to_errno;
#[cfg(target_os = "linux")]
pub use fuse::mount_fuse;

#[cfg(target_os = "linux")]
pub use fuse::FuseClient;
