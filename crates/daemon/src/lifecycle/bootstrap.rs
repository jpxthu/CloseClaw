//! Bootstrap helpers: .env loading, bootstrap mode, config migration, debug log.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use crate::Daemon;
use closeclaw_debug_log::{DebugLog, DebugLogConfig};
use tracing::{debug, info, warn};

// --- Config loading helpers ---
impl Daemon {
    /// Load .env file from config_dir if it exists.
    pub(crate) fn load_env(config_dir: &str) {
        let env_path = std::path::Path::new(config_dir).join(".env");
        if env_path.exists() {
            if let Err(e) = crate::load_env_file(&env_path) {
                warn!(error = %e, path = %env_path.display(), "failed to load .env file");
            } else {
                info!("Loaded environment from {}", env_path.display());
            }
        }
    }

    /// Read BOOTSTRAP_MODE env var and convert to BootstrapMode.
    /// "minimal" → Minimal, anything else (including absent) → Full.
    #[allow(dead_code)]
    pub(crate) fn read_bootstrap_mode() -> closeclaw_session::bootstrap::BootstrapMode {
        match std::env::var("BOOTSTRAP_MODE").as_deref() {
            Ok("minimal") => closeclaw_session::bootstrap::BootstrapMode::Minimal,
            _ => closeclaw_session::bootstrap::BootstrapMode::Full,
        }
    }

    /// Migrate legacy openclaw.json if present (non-fatal on error).
    pub(crate) fn run_config_migration(config_dir: &str) {
        let openclaw_json_path = std::path::Path::new(config_dir).join("openclaw.json");
        info!("Checking for legacy openclaw.json migration...");
        match closeclaw_config::migration::migrate_if_needed(&openclaw_json_path, config_dir) {
            Ok(true) => info!("Legacy openclaw.json migration completed successfully"),
            Ok(false) => info!("No migration needed — config directory is up to date"),
            Err(e) => warn!(
                error = %e,
                "openclaw.json migration failed — continuing with existing config"
            ),
        }
    }

    /// Initialize the debug log framework from config.
    ///
    /// Reads `{config_dir}/config/debug_log.json`. If the file is missing
    /// or invalid, returns `None` — the daemon continues without debug logging.
    pub(super) async fn init_debug_log(config_dir: &str) -> Option<DebugLog> {
        let config_path = std::path::Path::new(config_dir)
            .join("config")
            .join("debug_log.json");
        if !config_path.exists() {
            debug!("debug_log.json not found — skipping debug log init");
            return None;
        }
        match DebugLogConfig::from_file(&config_path).await {
            Ok(config) => match DebugLog::new(config).await {
                Ok(debug_log) => Some(debug_log),
                Err(e) => {
                    warn!(
                        error = %e,
                        "failed to create DebugLog instance — continuing without"
                    );
                    None
                }
            },
            Err(e) => {
                warn!(
                    error = %e,
                    path = %config_path.display(),
                    "failed to load debug_log.json — continuing without"
                );
                None
            }
        }
    }
}
