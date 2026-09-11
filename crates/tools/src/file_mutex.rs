//! Per-file mutex map for serializing concurrent writes to the same file.
//!
//! Re-exported from [`closeclaw_common::file_mutex`].

pub use closeclaw_common::file_mutex::{FileMutexMap, TryAcquireResult};
