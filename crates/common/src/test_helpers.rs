//! Shared test helpers for daemon, integration and cross-crate tests.

use std::io;
use std::sync::Mutex;

use crate::audit_log::{AuditLogEntry, AuditLogFilter, AuditLogger};

/// Create `dir` and any missing parents — the sanctioned way for tests to
/// add sub-directories **under** a `tempfile::TempDir` root.
///
/// `scripts/test-audit.sh` reports raw `std::fs::create_dir_all` /
/// `remove_dir_all` in scanned test files as manual directory management;
/// temp-tree layout expected by the daemon or fixtures is prepared through
/// this helper instead, so every created path stays inside the caller's
/// managed temp tree (STANDARDS §8).
///
/// Debug builds assert the TempDir contract: `dir` must resolve under
/// `std::env::temp_dir()` (where `tempfile::TempDir` roots live), so a
/// mis-injected path fails fast in tests instead of writing elsewhere.
pub fn ensure_dir(dir: &std::path::Path) -> io::Result<()> {
    debug_assert!(
        dir.starts_with(std::env::temp_dir()),
        "ensure_dir contract violation: {dir:?} is not under std::env::temp_dir(); \
         tests must operate inside a tempfile::TempDir root (STANDARDS §8)"
    );
    std::fs::create_dir_all(dir)
}

/// Write the config skeleton into `dir`: the 5 mandatory files
/// (channels.json, gateway.json, plugins.json, system.json,
/// accounts.json) plus a valid placeholder models.json.
///
/// Single implementation (Step 1.20): the mandatory-file half is
/// [`write_mandatory_without_models`], models.json is appended here.
pub fn write_mandatory_configs(dir: &std::path::Path) -> io::Result<()> {
    write_mandatory_without_models(dir)?;
    std::fs::write(
        dir.join("models.json"),
        serde_json::json!({"version": "1.0"}).to_string(),
    )
}

/// Write only the 5 mandatory config files into `dir` — models.json is
/// deliberately absent, exercising the optional-section semantics
/// (missing models.json never gates startup: design doc
/// `docs/design/daemon/README.md` 「models.json 缺失…系统仍正常启动」).
pub fn write_mandatory_without_models(dir: &std::path::Path) -> io::Result<()> {
    for name in &[
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )?;
    }
    Ok(())
}

/// In-memory [`AuditLogger`] mock shared by tests across crates.
///
/// Stores entries in insertion order — read the full log through
/// [`MemAuditLogger::entries`] — and answers
/// [`AuditLogger::query_entries`] per the trait contract: keep entries
/// matching `filter`, newest first. Newest-first compares parsed RFC 3339
/// instants — the same basis as [`AuditLogFilter::matches`]' since/until
/// window, so mixed `Z` / `+08:00` / `+00:00` forms order by absolute
/// time instead of lexicographically; an entry whose timestamp fails to
/// parse has no comparable instant and sorts last.
///
/// Consumers: the trait-contract test in `crate::audit_log`, plus the
/// permission crate's approval-flow and engine audit-log tests
/// (previously three verbatim-duplicated in-memory mocks).
#[derive(Default)]
pub struct MemAuditLogger {
    entries: Mutex<Vec<AuditLogEntry>>,
}

impl MemAuditLogger {
    /// Create an empty logger.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of every logged entry, in insertion order.
    pub fn entries(&self) -> Vec<AuditLogEntry> {
        self.entries.lock().unwrap().clone()
    }
}

impl AuditLogger for MemAuditLogger {
    fn log(&self, entry: &AuditLogEntry) {
        self.entries.lock().unwrap().push(entry.clone());
    }

    fn query_entries(&self, filter: &AuditLogFilter) -> Vec<AuditLogEntry> {
        let mut matched: Vec<AuditLogEntry> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();
        // Newest first by the parsed RFC 3339 instant — the same basis as
        // `AuditLogFilter::matches`' since/until window, so mixed ISO 8601
        // offset forms (`Z` / `+08:00` / `+00:00`) order by absolute time,
        // not by lexicographic string order. `None` (unparseable timestamp)
        // sorts last; `sort_by_key` is stable, so entries at the same
        // instant (and the trailing unparseable group) keep insertion order.
        matched.sort_by_key(|e| {
            chrono::DateTime::parse_from_rfc3339(&e.timestamp)
                .map(std::cmp::Reverse)
                .ok()
        });
        matched
    }
}
