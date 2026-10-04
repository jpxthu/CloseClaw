//! Shared test helpers for daemon and integration tests.

use std::io;

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
