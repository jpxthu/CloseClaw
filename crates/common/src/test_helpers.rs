//! Shared test helpers for daemon and integration tests.

use std::io;

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
