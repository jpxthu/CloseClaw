//! Permission engine and audit logger configuration helpers.
//!
//! Extracted from `lifecycle.rs` to keep source files within the
//! 1000-line CONTRIBUTING.md limit.

use super::Daemon;
use closeclaw_config::SystemConfigData;
use closeclaw_permission::engine::audit_log::{AuditLogger, FileAuditLogger};
use closeclaw_permission::engine::rejection_log::FileRejectionLogger;
use closeclaw_permission::{Defaults, PermissionEngine, RuleSet};
use std::sync::Arc;
use tracing::info;

impl Daemon {
    /// Build permission engine, loading templates from config_dir/templates/ if present.
    ///
    /// When a `rejection_log` section is present in `system.json`, a
    /// [`FileRejectionLogger`] with the configured `max_entries` limit is
    /// injected via [`PermissionEngine::with_rejection_logger`].
    pub(crate) fn build_permission_engine(
        config_dir: &str,
        audit_logger: Option<Arc<dyn AuditLogger>>,
    ) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
        let rule_set = RuleSet {
            rules: Vec::new(),
            defaults: Defaults::default(),
            user_defaults: Defaults::user_defaults(),
            template_includes: Vec::new(),
            rule_version: String::new(),
        };
        let mut engine = PermissionEngine::new(rule_set, std::path::PathBuf::from(config_dir));
        let templates_dir = std::path::Path::new(config_dir).join("templates");
        if templates_dir.exists() {
            if let Ok(templates) =
                closeclaw_permission::templates::load_templates_from_dir(&templates_dir)
            {
                let count = templates.len();
                if count > 0 {
                    engine.load_templates(templates);
                    info!(
                        "Loaded {} permission templates from {}",
                        count,
                        templates_dir.display()
                    );
                }
            }
        }
        let engine = Self::wire_rejection_logger(engine, config_dir);
        let engine = if let Some(logger) = audit_logger {
            engine.with_audit_logger(logger)
        } else {
            engine
        };
        info!("Permission engine initialized");
        Arc::new(tokio::sync::RwLock::new(engine))
    }

    /// Read `rejection_log` config from `system.json` and inject the
    /// logger into the permission engine.
    fn wire_rejection_logger(mut engine: PermissionEngine, config_dir: &str) -> PermissionEngine {
        let system_path = std::path::Path::new(config_dir).join("system.json");
        if !system_path.exists() {
            return engine;
        }
        match SystemConfigData::from_file(&system_path) {
            Ok(sys_cfg) => {
                if let Some(rejection_cfg) = sys_cfg.rejection_log {
                    let log_path = std::path::Path::new(config_dir)
                        .join("logs")
                        .join("rejection.log");
                    match FileRejectionLogger::new_with_limit(log_path, rejection_cfg.max_entries) {
                        Ok(logger) => {
                            engine = engine.with_rejection_logger(Arc::new(logger));
                            info!(
                                max_entries = ?rejection_cfg.max_entries,
                                "Rejection log logger configured"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to create rejection log logger \
                                 — continuing without"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "system.json not found or invalid — skipping \
                     rejection log config"
                );
            }
        }
        engine
    }

    /// Read `audit_log` config from `system.json` and create a
    /// [`FileAuditLogger`] if configured.
    pub(crate) fn create_audit_logger(config_dir: &str) -> Option<Arc<dyn AuditLogger>> {
        let system_path = std::path::Path::new(config_dir).join("system.json");
        if !system_path.exists() {
            return None;
        }
        match SystemConfigData::from_file(&system_path) {
            Ok(sys_cfg) => {
                if let Some(audit_cfg) = sys_cfg.audit_log {
                    let log_path = std::path::Path::new(config_dir)
                        .join("logs")
                        .join("audit.log");
                    match FileAuditLogger::new_with_limit(log_path, audit_cfg.max_entries) {
                        Ok(logger) => {
                            info!(
                                max_entries = ?audit_cfg.max_entries,
                                "Audit log logger configured"
                            );
                            Some(Arc::new(logger))
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to create audit log logger \
                                 — continuing without"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "system.json not found or invalid — skipping \
                     audit log config"
                );
                None
            }
        }
    }
}
