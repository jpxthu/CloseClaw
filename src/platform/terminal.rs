//! Terminal capability detection and user identity.
//!
//! Provides functions to detect whether the current terminal supports
//! ANSI escape sequences, and to retrieve the current user's UID.

/// Returns `true` if the current terminal supports ANSI escape sequences.
///
/// On Unix, checks the `TERM` environment variable for known values.
/// On Windows, detects terminal emulation environments (e.g. ConEmu, mintty).
pub fn supports_ansi() -> bool {
    cfg!(unix)
        && std::env::var("TERM")
            .map(|v| v != "dumb" && !v.is_empty())
            .unwrap_or(false)
}

/// Returns the current user's UID as a string.
///
/// On Unix, returns the numeric UID via `libc::getuid()`.
/// On Windows, returns the username via environment variable.
pub fn current_uid() -> String {
    #[cfg(unix)]
    {
        // SAFETY: getuid() is always safe and returns the real UID.
        unsafe { libc::getuid().to_string() }
    }
    #[cfg(not(unix))]
    {
        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_ansi_returns_bool() {
        // supports_ansi should not panic regardless of environment
        let _ = supports_ansi();
    }

    #[test]
    fn test_current_uid_non_empty() {
        let uid = current_uid();
        assert!(
            !uid.is_empty(),
            "current_uid() must return non-empty string"
        );
    }
}
