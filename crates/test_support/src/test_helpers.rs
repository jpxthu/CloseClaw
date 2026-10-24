//! Local helpers for `closeclaw-test-support`'s own unit tests.

use std::io;

/// Create `dir` and any missing parents — the sanctioned way for this
/// crate's own tests to add sub-directories **under** a
/// `tempfile::TempDir` root (STANDARDS §8).
///
/// Local equivalent of `closeclaw_common::test_helpers::ensure_dir`: this
/// crate does not depend on closeclaw-common, so the helper stays a
/// verbatim copy (same debug_assert contract) until a cross-crate edge is
/// justified.
pub(crate) fn ensure_dir(dir: &std::path::Path) -> io::Result<()> {
    debug_assert!(
        dir.starts_with(std::env::temp_dir()),
        "ensure_dir contract violation: {dir:?} is not under std::env::temp_dir(); \
         tests must operate inside a tempfile::TempDir root (STANDARDS §8)"
    );
    std::fs::create_dir_all(dir)
}
