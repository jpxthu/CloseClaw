use crate::manager::{ConfigManager, ConfigSection};
use crate::reload_manager::{
    dispatch_change, is_credentials_path, ConfigReloadManager, ReloadCallback,
};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Mock callback that records reload invocations.
struct MockCallback {
    agents_called: std::sync::atomic::AtomicBool,
    permissions_called: std::sync::atomic::AtomicBool,
}

impl MockCallback {
    fn new() -> Self {
        Self {
            agents_called: std::sync::atomic::AtomicBool::new(false),
            permissions_called: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn was_agents_called(&self) -> bool {
        self.agents_called
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn was_permissions_called(&self) -> bool {
        self.permissions_called
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ReloadCallback for MockCallback {
    fn on_agents_changed(&self, _path: &Path, _cm: &ConfigManager) {
        self.agents_called
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn on_permissions_changed(&self, _path: &Path, _cm: &ConfigManager) {
        self.permissions_called
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn on_session_reloaded(&self, _cm: &ConfigManager) {}
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
// Step 1.4 — Credentials hot-reload behavioral dimension tests
// ------------------------------------------------------------------

// -- Happy path: credentials change → validation passes → staged --

/// Happy path: credentials file change → validation passes → new
/// value staged for restart, runtime retains old value.
#[test]
fn test_credentials_happy_path_staging_and_runtime_preserved() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"sk-old"}"#,
    )
    .unwrap();

    // First load to establish a runtime value
    mgr.reload_credentials().unwrap();
    cm.apply_pending_restart();
    let runtime_before = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(runtime_before["providers"]["openai"]["apiKey"], "sk-old");

    // Write new credentials file
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"sk-new"}"#,
    )
    .unwrap();

    // Reload
    mgr.reload_credentials().unwrap();

    // Runtime must still expose the old value
    let runtime_after = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(
        runtime_after["providers"]["openai"]["apiKey"], "sk-old",
        "runtime must retain old value after credentials reload"
    );

    // Staged value must hold the new value
    let staged = cm
        .pending_restart_value(ConfigSection::Credentials)
        .expect("should have staged restart value");
    assert_eq!(
        staged["providers"]["openai"]["apiKey"], "sk-new",
        "staged value should hold the new credentials"
    );
}

// -- Path attribution boundary: deep subdirectory + runtime-added --

/// Credentials files in nested subdirectories of credentials/ should
/// be correctly attributed via is_credentials_path.
#[test]
fn test_credentials_deep_subdirectory_path_attribution() {
    // Three levels deep
    assert!(is_credentials_path(Path::new(
        "/config/credentials/vendor/sub/deep.json"
    )));
    // Exactly at credentials/ boundary (no trailing slash content)
    assert!(is_credentials_path(Path::new(
        "/config/credentials/file.json"
    )));
    // Edge: 'credentials' as filename (not directory)
    assert!(!is_credentials_path(Path::new("/config/credentials")));
    // Edge: similar name but not credentials
    assert!(!is_credentials_path(Path::new(
        "/config/credentials_backup/file.json"
    )));
}

/// A file created at runtime inside credentials/ should be correctly
/// dispatched to the credentials reload path.
#[test]
fn test_credentials_runtime_added_file_dispatched_to_credentials_reload() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    // Create credentials dir (empty initially)
    std::fs::create_dir_all(d.path().join("credentials")).unwrap();

    // Simulate runtime-added file
    std::fs::write(
        d.path().join("credentials/new_provider.json"),
        r#"{"provider":"openai","apiKey":"sk-new"}"#,
    )
    .unwrap();

    let path = d.path().join("credentials/new_provider.json");
    dispatch_change(&path, &mgr);

    // Should NOT trigger agents or permissions callbacks
    assert!(!cb.was_agents_called());
    assert!(!cb.was_permissions_called());

    // The staged value should contain the new provider
    let staged = cm
        .pending_restart_value(ConfigSection::Credentials)
        .expect("runtime-added file should trigger credentials staging");
    assert_eq!(staged["providers"]["openai"]["apiKey"], "sk-new");
}

// -- No old value scenario --

/// When no previous credentials value exists in memory and the first
/// load succeeds, the value is staged for restart (no runtime value).
#[test]
fn test_credentials_no_old_value_first_load_success_stages() {
    let d = TempDir::new().unwrap();
    // Use a bare ConfigManager (no load) to ensure no old value
    let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    std::fs::create_dir_all(d.path().join("credentials")).unwrap();
    std::fs::write(
        d.path().join("credentials/anthropic.json"),
        r#"{"provider":"anthropic","apiKey":"sk-ant"}"#,
    )
    .unwrap();

    mgr.reload_credentials().unwrap();

    // Runtime should have no credentials value (first load)
    let runtime = cm.get_section_value(ConfigSection::Credentials);
    assert!(
        runtime.is_none(),
        "runtime must have no credentials value on first load"
    );

    // Value should be staged for restart
    let staged = cm
        .pending_restart_value(ConfigSection::Credentials)
        .expect("first load should stage for restart");
    assert_eq!(staged["providers"]["anthropic"]["apiKey"], "sk-ant");
}

/// When no previous value exists and credentials validation fails
/// (no valid credentials in directory), the staged value should be
/// empty and runtime remains None.
#[test]
fn test_credentials_no_old_value_empty_dir_stages_empty() {
    let d = TempDir::new().unwrap();
    let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    std::fs::create_dir_all(d.path().join("credentials")).unwrap();
    // Write a malformed file that will be skipped by load_from_dir
    std::fs::write(d.path().join("credentials/bad.json"), r#"not valid json"#).unwrap();

    mgr.reload_credentials().unwrap();

    // Runtime remains None
    assert!(cm.get_section_value(ConfigSection::Credentials).is_none());

    // Staged value should be empty providers
    let staged = cm
        .pending_restart_value(ConfigSection::Credentials)
        .expect("should stage empty providers");
    assert_eq!(staged["providers"].as_object().unwrap().len(), 0);
}

// -- Regression: non-credentials sections unaffected --

/// Reloading credentials should not affect the models.json section.
#[test]
fn test_credentials_reload_does_not_affect_models_section() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    let models_before = cm.section(ConfigSection::Models).unwrap();

    std::fs::create_dir_all(d.path().join("credentials")).unwrap();
    std::fs::write(
        d.path().join("credentials/openai.json"),
        r#"{"provider":"openai","apiKey":"sk-test"}"#,
    )
    .unwrap();

    mgr.reload_credentials().unwrap();

    let models_after = cm.section(ConfigSection::Models).unwrap();
    assert_eq!(
        models_before, models_after,
        "models section should be unchanged after credentials reload"
    );
}

/// Reloading a non-credentials section (models.json) should still
/// work correctly and not interfere with credentials.
#[test]
fn test_models_reload_unaffected_by_credentials() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

    // First establish credentials
    std::fs::create_dir_all(d.path().join("credentials")).unwrap();
    std::fs::write(
        d.path().join("credentials/openai.json"),
        r#"{"provider":"openai","apiKey":"sk-old"}"#,
    )
    .unwrap();
    mgr.reload_credentials().unwrap();
    cm.apply_pending_restart();

    // Reload models.json
    std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();
    mgr.reload_section(ConfigSection::Models).unwrap();

    // Models should be updated (restart-class → staged)
    let models_pending = cm
        .pending_restart_value(ConfigSection::Models)
        .expect("models should have staged value");
    assert_eq!(models_pending["models"][0]["id"], "m1");

    // Credentials runtime should still be the old value
    let creds_runtime = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(creds_runtime["providers"]["openai"]["apiKey"], "sk-old");
}
