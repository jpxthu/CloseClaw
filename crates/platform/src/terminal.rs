//! Terminal capability detection, user identity, and I/O.
//!
//! Provides functions to detect terminal capabilities, retrieve the
//! current user's UID, and perform cross-platform terminal I/O.

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

/// Check if stdin is attached to a terminal (TTY).
///
/// Returns `true` if stdin is a terminal device, `false` if it is a
/// pipe or redirected file.
pub fn is_terminal() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: isatty(2) is always safe and does not modify state.
        unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
    }
    #[cfg(not(unix))]
    {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal()
    }
}

/// Read a line of input from stdin.
///
/// Returns the line content without the trailing newline character.
/// Returns an error if stdin cannot be read.
pub fn read_line_raw() -> anyhow::Result<String> {
    use std::io::{self, BufRead};
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches('\n').to_string())
}

/// Write raw bytes to stdout.
///
/// Flushes stdout after writing to ensure output is immediately
/// visible.
pub fn write_raw(data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(data)?;
    handle.flush()?;
    Ok(())
}
