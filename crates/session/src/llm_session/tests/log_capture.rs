//! Shared log-capture helper for `llm_session` tests, extracted from
//! `tests/mod.rs` (issue #3112) so that `mod.rs` keeps only module
//! declarations (CONTRIBUTING.md §模块).
//!
//! `tests/mod.rs` re-exports [`capture_logs`]
//! (`use self::log_capture::capture_logs;`), so callers keep writing
//! `use super::capture_logs;` unchanged; `VecWriter` is not re-exported
//! (no caller references it).
//!
//! Constraint: every calling test **must** carry `#[serial_test::serial]`
//! — see the `# Concurrency` section on [`capture_logs`].

use std::sync::{Arc, Mutex};

/// A `MakeWriter` that clones an `Arc<Mutex<Vec<u8>>>` buffer so the
/// subscriber can write into it while the caller keeps a handle to read
/// the captured bytes back.
#[derive(Clone, Default)]
pub(super) struct VecWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for VecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for VecWriter {
    type Writer = VecWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Install a thread-local fmt subscriber (no target/ansi) filtered to
/// `level`, writing into an in-memory buffer; run `f`, drop the
/// subscriber guard so the buffer is flushed, and return the closure's
/// result together with the captured log output. Shared log-capture
/// helper for session tests (issue #3107).
///
/// # Level (difference from config's `capture_warn_logs`)
///
/// config's helper fixes the filter at `WARN`; this one takes the level
/// as a parameter because session tests assert both `info`-level events
/// (e.g. `skill_listing_injection`) and `warn`-level events (e.g. cache
/// break), and a fixed `WARN` filter cannot capture the former.
///
/// # Concurrency
///
/// Caller tests **must** carry `#[serial_test::serial]`. Installing the
/// subscriber registers callsites in the process-global tracing
/// callsite-interest cache; concurrent registration on the same callsite
/// can drop events and empty the capture buffer (issue #3102 race).
pub(super) fn capture_logs<T>(f: impl FnOnce() -> T, level: tracing::Level) -> (T, String) {
    let buffer = VecWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(level)
        .with_target(false)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let value = f();
    drop(guard);
    let logs = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    (value, logs)
}
