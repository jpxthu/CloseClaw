//! System temporary directory resolution.
//!
//! Provides a pure path query: resolves the absolute path of the system
//! temporary directory per platform convention without creating or
//! cleaning up the directory.

use std::path::PathBuf;

/// Returns the absolute path of the system temporary directory.
///
/// The path is resolved per platform convention:
///
/// | Platform | Resolution |
/// |----------|------------|
/// | Linux / WSL2 | `$TMPDIR`; falls back to the system default `/tmp` when unset |
/// | macOS | `$TMPDIR` (per-user session directory injected by launchd) |
/// | Windows | No native support; via WSL2 behaves the same as Linux |
///
/// The resolution basis is identical to [`std::env::temp_dir()`], so the
/// platform branching (including the `$TMPDIR` fallback semantics) is
/// delegated to the standard library.
///
/// This function only returns the path; it does not create or clean up the
/// directory. The system temporary directory is guaranteed to exist by the
/// operating system; writability and temporary file lifecycle are the
/// responsibility of the caller.
pub fn system_temp_dir() -> PathBuf {
    std::env::temp_dir()
}
