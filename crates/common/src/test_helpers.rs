//! Shared test helpers for daemon and integration tests.

use std::io;

/// Write the config skeleton into `dir`: the 5 mandatory files
/// (channels.json, gateway.json, plugins.json, system.json,
/// accounts.json) plus the optional models.json.
///
/// Reused across daemon unit tests, E2E tests, and integration tests
/// to avoid duplicating the same for-loop in every test helper.
pub fn write_mandatory_configs(dir: &std::path::Path) -> io::Result<()> {
    for name in &[
        "models.json",
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
