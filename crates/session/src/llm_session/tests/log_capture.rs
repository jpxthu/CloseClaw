//! Shared log-capture helper for `llm_session` tests, extracted from
//! `tests/mod.rs` (issue #3112) so that `mod.rs` keeps only module
//! declarations (CONTRIBUTING.md §模块).
//!
//! `tests/mod.rs` re-exports [`capture_logs`]
//! (`use self::log_capture::capture_logs;`), so callers keep writing
//! `use super::capture_logs;` unchanged; `VecWriter` is not
//! re-exported (referenced only by `log_capture_tests`, for its
//! `is::<CaptureSubscriber>()` guard discrimination).
//!
//! Two capture entry points coexist long-term: synchronous
//! [`capture_logs`] for tests with synchronous bodies, and async
//! [`capture_logs_async`] for tests whose body awaits (see its docs for
//! the threading contract).
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
    let guard = install(&buffer, level);
    let value = f();
    let logs = drain(&buffer, guard);
    (value, logs)
}

/// Async twin of [`capture_logs`]: same subscriber setup, but `f`
/// builds a future that is awaited to completion instead of running a
/// synchronous closure; returns `(T, String)` with the future's output
/// and the logs captured up to the await's end. Lets async tests use
/// `#[tokio::test]` directly instead of the `Runtime::new()` +
/// `rt.block_on(...)` boilerplate (issue #3168).
///
/// `f: impl FnOnce() -> Fut` (not an async closure) for Rust-version
/// compatibility; call it as
/// `capture_logs_async(|| async { ... }, level).await`.
///
/// # Threading contract (caller must satisfy)
///
/// - **`#[tokio::test]` with the default current-thread flavor.** The
///   `set_default` guard is thread-local: it installs the subscriber
///   on the thread that runs `f().await`'s first poll and can only be
///   uninstalled on that same thread, so the future must stay on one
///   thread. Two real risks if it does not (the spawn-attribution
///   pair measured in `log_capture_tests.rs`): a **multi-thread
///   flavor** can migrate the future — or a `tokio::spawn`ed child —
///   onto another thread, so the guard drops away from the installing
///   thread and the buffer is never released; a **child task
///   outliving the capture scope** keeps running after the guard
///   drops, so its events are missed. `tokio::spawn` inside the
///   capture scope is therefore not itself a violation: awaited
///   *inside* the scope on the current-thread runtime, the runtime
///   polls the test future and the child on the same thread, so the
///   child's events reach the same buffer (the measured basis
///   recorded in `log_capture_tests.rs`). Blocking std calls (e.g.
///   `recv_timeout`) must not appear in the async body either: there
///   is no other thread to poll it.
/// - **`#[serial_test::serial]`**, for the same callsite-interest
///   cache reason as [`capture_logs`] (issue #3102 race): concurrent
///   registration on the same callsite can drop events and empty the
///   buffer.
///
/// # Relation to [`capture_logs`]
///
/// The two helpers coexist long-term: this one covers async test
/// bodies, while the sync version keeps its existing call sites
/// (and sync tests) unchanged.
pub(super) async fn capture_logs_async<T, Fut>(
    f: impl FnOnce() -> Fut,
    level: tracing::Level,
) -> (T, String)
where
    Fut: std::future::Future<Output = T>,
{
    let buffer = VecWriter::default();
    let guard = install(&buffer, level);
    let tid = std::thread::current().id();
    let value = f().await;
    assert_eq!(
        std::thread::current().id(),
        tid,
        "capture_logs_async must run on a single thread: the set_default guard \
         is thread-affine, so a future that migrated across threads cannot be \
         unloaded on the installing thread; drive it with the current-thread \
         #[tokio::test] flavor"
    );
    let logs = drain(&buffer, guard);
    (value, logs)
}

/// Shared tail of [`capture_logs`] and [`capture_logs_async`]: drop
/// `guard` so the subscriber is uninstalled and stops writing, then
/// read back everything it captured as UTF-8 text.
fn drain(buffer: &VecWriter, guard: tracing::subscriber::DefaultGuard) -> String {
    drop(guard);
    String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
}

/// Build the fmt subscriber (no target/ansi) filtered to `level`,
/// writing into `buffer`, and install it as this thread's default
/// subscriber; dropping the returned guard uninstalls it so the
/// buffer stops being written and can be read back.
fn install(buffer: &VecWriter, level: tracing::Level) -> tracing::subscriber::DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(level)
        .with_target(false)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_default(subscriber)
}
