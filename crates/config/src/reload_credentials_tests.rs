use crate::manager::{ConfigManager, ConfigSection};
use crate::reload_manager::{
    dispatch_change, is_credentials_path, ConfigReloadManager, ReloadCallback,
};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Mock callback that records reload and validation-failure invocations.
struct MockCallback {
    agents_called: std::sync::atomic::AtomicBool,
    permissions_called: std::sync::atomic::AtomicBool,
    validation_failed: std::sync::atomic::AtomicBool,
    last_validation_error: std::sync::Mutex<Option<String>>,
}

impl MockCallback {
    fn new() -> Self {
        Self {
            agents_called: std::sync::atomic::AtomicBool::new(false),
            permissions_called: std::sync::atomic::AtomicBool::new(false),
            validation_failed: std::sync::atomic::AtomicBool::new(false),
            last_validation_error: std::sync::Mutex::new(None),
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

    fn was_validation_failed_called(&self) -> bool {
        self.validation_failed
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

    fn on_validation_failed(
        &self,
        _section: ConfigSection,
        _path: &Path,
        error: &str,
        _config_manager: &ConfigManager,
    ) {
        self.validation_failed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *self.last_validation_error.lock().unwrap() = Some(error.to_string());
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

/// When no previous value exists and credentials parsing fails
/// (malformed JSON), the load should fail, no value should be staged,
/// and the validation-failed callback should be invoked.
#[test]
fn test_credentials_no_old_value_parse_failure_blocks_load() {
    let d = TempDir::new().unwrap();
    let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    std::fs::create_dir_all(d.path().join("credentials")).unwrap();
    std::fs::write(d.path().join("credentials/bad.json"), r#"not valid json"#).unwrap();

    let result = mgr.reload_credentials();
    assert!(result.is_err(), "should fail on malformed JSON");
    assert!(
        cb.was_validation_failed_called(),
        "on_validation_failed should be called"
    );

    // Runtime remains None — no value staged
    assert!(cm.get_section_value(ConfigSection::Credentials).is_none());
    assert!(
        cm.pending_restart_value(ConfigSection::Credentials)
            .is_none(),
        "no value should be staged on failure"
    );
}

/// When credentials exist with a valid old value and reload encounters
/// a parse failure, runtime must retain the old value and no new
/// value should be staged.
#[test]
fn test_credentials_old_value_retained_on_parse_failure() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"sk-old"}"#,
    )
    .unwrap();

    // Establish runtime value
    mgr.reload_credentials().unwrap();
    cm.apply_pending_restart();
    let runtime_before = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(runtime_before["providers"]["openai"]["apiKey"], "sk-old");

    // Now write a malformed file
    std::fs::write(creds_dir.join("openai.json"), r#"{broken"#).unwrap();

    let result = mgr.reload_credentials();
    assert!(result.is_err(), "should fail on malformed JSON");
    assert!(cb.was_validation_failed_called());

    // Runtime must still expose old value
    let runtime_after = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(
        runtime_after["providers"]["openai"]["apiKey"], "sk-old",
        "runtime must retain old value after parse failure"
    );
    // No new value staged
    assert!(
        cm.pending_restart_value(ConfigSection::Credentials)
            .is_none(),
        "no value should be staged on parse failure"
    );
}

/// Validation failure (valid JSON but structurally invalid credential)
/// should trigger callback, retain old runtime value, and not stage.
#[test]
fn test_credentials_validation_failure_retains_old_value() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"sk-old"}"#,
    )
    .unwrap();

    // Establish runtime value
    mgr.reload_credentials().unwrap();
    cm.apply_pending_restart();

    // Write a structurally invalid credential (empty apiKey)
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":""}"#,
    )
    .unwrap();

    let result = mgr.reload_credentials();
    assert!(result.is_err(), "should fail on validation error");
    assert!(cb.was_validation_failed_called());

    // Runtime must still expose old value
    let runtime_after = cm.section(ConfigSection::Credentials).unwrap();
    assert_eq!(
        runtime_after["providers"]["openai"]["apiKey"], "sk-old",
        "runtime must retain old value after validation failure"
    );
    assert!(cm
        .pending_restart_value(ConfigSection::Credentials)
        .is_none());
}

/// When no old value exists and credential validation fails (e.g.,
/// empty apiKey), the load should be blocked per the design-doc
/// "no old value → block" semantics.
#[test]
fn test_credentials_no_old_value_validation_failure_blocks() {
    let d = TempDir::new().unwrap();
    let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":""}"#,
    )
    .unwrap();

    let result = mgr.reload_credentials();
    assert!(result.is_err(), "should fail on validation error");
    assert!(cb.was_validation_failed_called());
    assert!(cm.get_section_value(ConfigSection::Credentials).is_none());
    assert!(cm
        .pending_restart_value(ConfigSection::Credentials)
        .is_none());
}

/// credential_path reference in models.json that points to a file
/// with a parse error should abort the load (file exists but invalid).
/// The bad file must be outside credentials/ so that load_from_dir_strict
/// does not catch it first — only the credential_path reference path is tested.
#[test]
fn test_credentials_credential_path_parse_error_aborts_load() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    // credentials/ dir exists but is empty (no files inside to trigger
    // load_from_dir_strict failure — we only test the credential_path path)
    std::fs::create_dir_all(d.path().join("credentials")).unwrap();

    // Place a valid credential file OUTSIDE credentials/, referenced via
    // models.json credentialPath. Use an absolute path because the models
    // validator resolves credentialPath relative to CWD.
    let external_dir = d.path().join("external_creds");
    std::fs::create_dir_all(&external_dir).unwrap();
    let cred_file = external_dir.join("openai.json");
    std::fs::write(
        &cred_file,
        r#"{"provider":"openai","apiKey":"sk-placeholder"}"#,
    )
    .unwrap();
    let abs_cred_path = cred_file.to_string_lossy();
    let models_json = format!(
        r#"{{"providers":{{"openai":{{"credentialPath":"{}","models":[]}}}}}}"#,
        abs_cred_path
    );
    std::fs::write(d.path().join("models.json"), &models_json).unwrap();
    mgr.reload_section(ConfigSection::Models).unwrap();
    // Apply pending restart so in-memory cache has the new models.json
    // with credentialPath (otherwise merge_credential_path_references
    // reads the old value without credentialPath).
    cm.apply_pending_restart();

    // Now overwrite the external file with malformed JSON
    std::fs::write(&cred_file, r#"{broken json"#).unwrap();

    let result = mgr.reload_credentials();
    assert!(
        result.is_err(),
        "should fail when credential_path file has parse error"
    );
    assert!(cb.was_validation_failed_called());
    assert!(cm
        .pending_restart_value(ConfigSection::Credentials)
        .is_none());
}

/// credential_path reference in models.json that points to a file
/// with validation error (empty apiKey) should abort the load.
/// The bad file must be outside credentials/ so that load_from_dir_strict
/// does not catch it first — only the credential_path reference path is tested.
#[test]
fn test_credentials_credential_path_validation_error_aborts_load() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    // credentials/ dir exists but is empty
    std::fs::create_dir_all(d.path().join("credentials")).unwrap();

    // Place a valid credential file OUTSIDE credentials/, referenced via
    // models.json credentialPath. Use an absolute path because the models
    // validator resolves credentialPath relative to CWD.
    let external_dir = d.path().join("external_creds");
    std::fs::create_dir_all(&external_dir).unwrap();
    let cred_file = external_dir.join("openai.json");
    std::fs::write(
        &cred_file,
        r#"{"provider":"openai","apiKey":"sk-placeholder"}"#,
    )
    .unwrap();
    let abs_cred_path = cred_file.to_string_lossy();
    let models_json = format!(
        r#"{{"providers":{{"openai":{{"credentialPath":"{}","models":[]}}}}}}"#,
        abs_cred_path
    );
    std::fs::write(d.path().join("models.json"), &models_json).unwrap();
    mgr.reload_section(ConfigSection::Models).unwrap();
    // Apply pending restart so in-memory cache has the new models.json
    // with credentialPath.
    cm.apply_pending_restart();

    // Now overwrite with valid JSON but invalid credential (empty apiKey)
    std::fs::write(&cred_file, r#"{"provider":"openai","apiKey":""}"#).unwrap();

    let result = mgr.reload_credentials();
    assert!(
        result.is_err(),
        "should fail when credential_path file has validation error"
    );
    assert!(cb.was_validation_failed_called());
    assert!(cm
        .pending_restart_value(ConfigSection::Credentials)
        .is_none());
}

/// credential_path reference to a missing file should warn but not
/// abort the load (missing ≠ invalid per design doc).
#[test]
fn test_credentials_credential_path_missing_file_warns_only() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();

    // Create valid credential file first so models.json validation passes.
    let cred_file = creds_dir.join("openai.json");
    std::fs::write(
        &cred_file,
        r#"{"provider":"openai","apiKey":"sk-placeholder"}"#,
    )
    .unwrap();
    let abs_cred_path = cred_file.to_string_lossy();
    let models_json = format!(
        r#"{{"providers":{{"openai":{{"credentialPath":"{}","models":[]}}}}}}"#,
        abs_cred_path
    );
    std::fs::write(d.path().join("models.json"), &models_json).unwrap();
    mgr.reload_section(ConfigSection::Models).unwrap();

    // Now delete the referenced file (simulating runtime deletion)
    std::fs::remove_file(&cred_file).unwrap();

    // Reload credentials — missing file should be warned-only
    let result = mgr.reload_credentials();
    assert!(
        result.is_ok(),
        "missing credential_path file should not abort"
    );
    assert!(!cb.was_validation_failed_called());
}

/// Mixed scenario: valid file + malformed file in credentials/ →
/// entire batch fails, first valid file is NOT staged.
#[test]
fn test_credentials_mixed_valid_and_invalid_batch_fails() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let cb = Arc::new(MockCallback::new());
    let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());

    let creds_dir = d.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();

    // One valid file
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"sk-good"}"#,
    )
    .unwrap();
    // One malformed file
    std::fs::write(creds_dir.join("bad.json"), r#"{broken"#).unwrap();

    let result = mgr.reload_credentials();
    assert!(
        result.is_err(),
        "batch should fail when any file is invalid"
    );
    assert!(cb.was_validation_failed_called());
    assert!(cm
        .pending_restart_value(ConfigSection::Credentials)
        .is_none());
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
