use crate::manager::{ConfigManager, ConfigSection};
use crate::reload_manager::{ConfigReloadManager, ReloadCallback};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Mock callback that tracks on_validation_failed invocations.
struct ValidationTrackingCallback {
    validation_failed: std::sync::atomic::AtomicBool,
    last_error: std::sync::Mutex<Option<String>>,
}

impl ValidationTrackingCallback {
    fn new() -> Self {
        Self {
            validation_failed: std::sync::atomic::AtomicBool::new(false),
            last_error: std::sync::Mutex::new(None),
        }
    }

    fn was_validation_failed_called(&self) -> bool {
        self.validation_failed
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ReloadCallback for ValidationTrackingCallback {
    fn on_agents_changed(&self, _path: &Path, _cm: &ConfigManager) {}
    fn on_permissions_changed(&self, _path: &Path, _cm: &ConfigManager) {}
    fn on_session_reloaded(&self, _cm: &ConfigManager) {}

    fn on_validation_failed(
        &self,
        _section: ConfigSection,
        _path: &Path,
        error: &str,
        _config_manager: &ConfigManager,
    ) {
        self.validation_failed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *self.last_error.lock().unwrap() = Some(error.to_string());
    }
}

fn make_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
    let sections = [
        ("models.json", r#"{"models":[]}"#),
        ("channels.json", r#"{"channels":{}}"#),
        ("gateway.json", r#"{"port":8080}"#),
        ("plugins.json", r#"{"plugins":[]}"#),
        ("system.json", r#"{"version":"1"}"#),
        ("accounts.json", r#"{"accounts":[]}"#),
        (
            "session.json",
            r#"{"defaults":{},"agents":{},"sweeperIntervalSeconds":600}"#,
        ),
    ];
    for (name, content) in &sections {
        std::fs::write(dir.join(name), content).unwrap();
    }
    let cm = ConfigManager::new(dir.to_path_buf()).unwrap();
    cm.load().unwrap();
    Arc::new(cm)
}

// ------------------------------------------------------------------
// Step 1.5 — restart-class staging via reload_section
// ------------------------------------------------------------------

#[test]
fn test_reload_section_restart_class_stages_to_pending() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    assert_eq!(cm.section(ConfigSection::Gateway).unwrap()["port"], 8080);
    std::fs::write(d.path().join("gateway.json"), r#"{"port":9999}"#).unwrap();
    mgr.reload_section(ConfigSection::Gateway).unwrap();

    // Runtime retains old value
    assert_eq!(cm.section(ConfigSection::Gateway).unwrap()["port"], 8080);
    // Pending holds new value
    assert_eq!(
        cm.pending_restart_value(ConfigSection::Gateway).unwrap()["port"],
        9999
    );
}

#[test]
fn test_reload_section_models_stages_to_pending() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"new"}]}"#).unwrap();
    mgr.reload_section(ConfigSection::Models).unwrap();

    let runtime = cm.section(ConfigSection::Models).unwrap();
    assert!(runtime
        .get("models")
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    let pending = cm.pending_restart_value(ConfigSection::Models).unwrap();
    assert_eq!(pending["models"][0]["id"], "new");
}

#[test]
fn test_reload_section_non_restart_class_updates_runtime() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    std::fs::write(d.path().join("system.json"), r#"{"version":"9.9"}"#).unwrap();
    mgr.reload_section(ConfigSection::System).unwrap();

    assert_eq!(cm.section(ConfigSection::System).unwrap()["version"], "9.9");
    assert!(cm.pending_restart_value(ConfigSection::System).is_none());
}

// ------------------------------------------------------------------
// Step 1.5 — on_validation_failed callback invocation
// ------------------------------------------------------------------

#[test]
fn test_on_validation_failed_called_on_parse_error() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    std::fs::write(d.path().join("system.json"), "not valid json!!!").unwrap();
    let result = mgr.reload_section(ConfigSection::System);
    assert!(result.is_err());
    assert!(cb.was_validation_failed_called());
    assert!(cb.last_error.lock().unwrap().is_some());
}

#[test]
fn test_on_validation_failed_called_on_validation_error() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    std::fs::write(d.path().join("models.json"), r#"{"models":"not array"}"#).unwrap();
    let result = mgr.reload_section(ConfigSection::Models);
    assert!(result.is_err());
    assert!(cb.was_validation_failed_called());
}

#[test]
fn test_on_validation_failed_not_called_on_success() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(ValidationTrackingCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    std::fs::write(d.path().join("system.json"), r#"{"version":"3.0"}"#).unwrap();
    mgr.reload_section(ConfigSection::System).unwrap();
    assert!(!cb.was_validation_failed_called());
}
