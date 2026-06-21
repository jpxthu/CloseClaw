//! Terminal capability detection and user identity.
//!
//! Provides functions to detect whether the current terminal supports
//! ANSI escape sequences, and to retrieve the current user's UID.

/// Returns `true` if the current terminal supports ANSI escape sequences.
///
/// On Windows, detects the Windows Terminal environment via `WT_SESSION`.
/// On all platforms, checks the `TERM` environment variable for known
/// ANSI-capable values (`xterm`, `screen`, `ansi`, `vt100`, `color`).
pub fn supports_ansi() -> bool {
    if std::env::var("WT_SESSION").is_ok() {
        return true;
    }
    std::env::var("TERM")
        .map(|term| {
            let t = term.to_lowercase();
            t.contains("xterm")
                || t.contains("screen")
                || t.contains("ansi")
                || t.contains("vt100")
                || t.contains("color")
        })
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
