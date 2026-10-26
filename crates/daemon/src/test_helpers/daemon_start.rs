//! Daemon-startup test fixtures: the isolated PID-file path and the
//! agents.json + mandatory-config writes required by
//! `Daemon::start_with_pid_file_path`.
//!
//! One crate-shared definition instead of a file-private copy per test
//! file (shutdown.rs precedent) — previously duplicated across
//! `tests.rs` and `lifecycle_run_tests.rs` (issue #4065).

use std::path::{Path, PathBuf};

use closeclaw_common::test_helpers::{ensure_dir, write_mandatory_without_models};

use crate::test_helpers::write_mandatory_configs;

/// Create an isolated PID file path under the given TempDir root:
/// `{root}/.closeclaw/daemon.pid` with the parent directory pre-created,
/// matching the production directory layout (same pattern as the cli
/// crate's `isolated_pid_file`). Keeps daemon PID-file side effects
/// inside the TempDir instead of the real `~/.closeclaw`.
pub(crate) fn isolated_pid_file(root: &Path) -> PathBuf {
    let pid_dir = root.join(".closeclaw");
    ensure_dir(&pid_dir).expect("failed to create .closeclaw dir");
    pid_dir.join("daemon.pid")
}

/// Create only `config/agents.json` (without mandatory config files)
/// in the given directory.
pub(crate) fn write_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    let agents_content = serde_json::json!({
        "version": "1.0",
        "agents": [
            {
                "name": "guide",
                "model": "minimax/MiniMax-M2",
                "persona": "test persona",
                "max_iterations": 10,
                "timeout_minutes": 5
            }
        ]
    });
    let config_dir = dir.join("config");
    ensure_dir(&config_dir)?;
    std::fs::write(config_dir.join("agents.json"), agents_content.to_string())?;
    Ok(())
}

/// Create a minimal agents.json, the 5 mandatory config files
/// (channels.json, gateway.json, plugins.json, system.json,
/// accounts.json) and the optional models.json in the given directory
/// so that `ConfigManager::load()` succeeds.
pub(crate) fn setup_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    write_agents_json(dir)?;
    // Mandatory configs go into the config/ subdirectory
    // (ConfigManager now receives <root>/config/ as config_dir)
    let config_dir = dir.join("config");
    write_mandatory_configs(&config_dir)?;
    Ok(())
}

/// Same as [`setup_agents_json`] but without models.json — the Models
/// section is optional and must not gate startup (design doc daemon
/// README: 「models.json 缺失…系统仍正常启动」).
pub(crate) fn setup_agents_json_without_models(dir: &std::path::Path) -> std::io::Result<()> {
    write_agents_json(dir)?;
    let config_dir = dir.join("config");
    write_mandatory_without_models(&config_dir)?;
    Ok(())
}
