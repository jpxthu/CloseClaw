//! Media store trait for cross-crate media access.
//!
//! Defines a trait that media storage implementations can provide,
//! allowing higher-layer crates (like gateway) to resolve media
//! references without depending on the concrete `MediaStore` type
//! from `im_adapter`.

use std::path::PathBuf;

use crate::MediaRef;

/// Errors from media store resolution.
#[derive(Debug, thiserror::Error)]
pub enum MediaStoreError {
    /// Media reference has no local path set.
    #[error("media reference has no path set")]
    NoPath,

    /// Resolved path does not exist on disk.
    #[error("media file not found: {0}")]
    FileNotFound(PathBuf),

    /// I/O error during file read.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Other implementation-specific error.
    #[error("{0}")]
    Other(String),
}

/// Trait for media store operations needed by the gateway.
///
/// Implemented by `closeclaw_im_adapter::media_store::MediaStore`.
/// Gateway depends only on this trait (via `closeclaw-common`),
/// avoiding a cyclic dependency with `im_adapter`.
pub trait MediaStoreAccess: Send + Sync {
    /// Resolve a [`MediaRef`] to its absolute local path.
    fn resolve_ref(&self, media_ref: &MediaRef) -> Result<PathBuf, MediaStoreError>;
}
