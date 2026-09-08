//! Write-with-readback: recover from transient write failures.
//!
//! When `std::fs::write` fails (e.g. timeout, interrupted), we read back the
//! file and compare it to the expected content. If it matches, the write is
//! treated as successful.

use std::path::Path;

use crate::ToolCallError;

/// Read back the file at `path` and compare to `expected`.
///
/// Returns `true` only when the file exists, is readable, and its entire
/// content equals `expected` byte-for-byte.
pub(super) fn is_recovered(path: &Path, expected: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

/// Write `content` to `path`, with readback recovery on transient failure.
///
/// - `std::fs::write` succeeds → `Ok(())`
/// - `std::fs::write` fails but `is_recovered` → `Ok(())`
/// - otherwise → `Err(ExecutionFailed)` with the original error message
pub(super) fn write_with_readback(path: &Path, content: &str) -> Result<(), ToolCallError> {
    match std::fs::write(path, content) {
        Ok(()) => Ok(()),
        Err(e) => {
            if is_recovered(path, content) {
                Ok(())
            } else {
                Err(ToolCallError::ExecutionFailed(format!(
                    "{}: {e}",
                    path.display()
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "readback_tests.rs"]
pub(crate) mod tests;
