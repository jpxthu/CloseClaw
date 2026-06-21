//! Shared test helpers for daemon and integration tests.

use std::io;

/// Write the 5 mandatory config files (models.json, channels.json,
/// gateway.json, plugins.json, system.json) into `dir`.
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
    ] {
        std::fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )?;
    }
    Ok(())
}
