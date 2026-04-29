//! Bootstrap Protection Layer — Compaction 防护机制
//!
//! Ensures that agent bootstrap files (AGENTS.md, SOUL.md, IDENTITY.md, USER.md)
//! are not summarization-distorted during OpenClaw session compaction.
//!
//! # Core Concept
//!
//! When OpenClaw triggers compaction on a long session, the bootstrap context
//! (injected at session start) gets summarized along with the transcript history.
//! This module provides:
//!
//! - [`BootstrapRegion`] — Marker structs delimiting bootstrap content in transcript
//! - [`BootstrapContext`] — Metadata tracking all bootstrap regions and their integrity
//! - [`BootstrapProtection`] — Main service for protecting/re-injecting bootstrap content
//!
//! # Usage
//!
//! 1. At session start: [`BootstrapProtection::protect_session`] to scan and mark bootstrap content
//! 2. Before compaction: [`BootstrapProtection::before_compact`] to store integrity hashes
//! 3. After compaction: [`BootstrapProtection::after_compact`] to detect corruption
//! 4. If corrupted: [`BootstrapProtection::reinject`] to prepend fresh bootstrap content

pub mod context;
pub mod helpers;
pub mod loader;
pub mod protection;
pub mod tests;
pub mod types;

// Re-exports for convenience
pub use context::BootstrapContext;
pub use loader::{bootstrap_file_list, load_bootstrap_files, BootstrapLoaderError, BootstrapMode};
pub use protection::BootstrapProtection;
pub use types::{
    BootstrapProtectionError, BootstrapRegion, BOOTSTRAP_REGION_END, BOOTSTRAP_REGION_START,
};
