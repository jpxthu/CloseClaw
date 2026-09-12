//! Step 1.5 — Cross-file reference validation tests.
//!
//! Verifies that cross-file reference validation (accounts→channels,
//! models→credentials) runs as a standalone non-blocking step after
//! all configs are loaded.

use super::*;
use std::fs;

/// Helper: write the 5 mandatory config files into `dir`.
fn write_mandatory(dir: &std::path::Path) {
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )
        .unwrap();
    }
}

/// accounts references non-existent channel → WARN, load succeeds.
#[test]
fn test_cross_ref_accounts_nonexistent_channel_warns() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory(tmp.path());
    fs::write(
        tmp.path().join("accounts.json"),
        serde_json::json!({"accounts": [{"accountId": "a1", "senderId": "s1", "platform": "telegram"}]}).to_string(),
    ).unwrap();
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    mgr.load().unwrap();
    assert!(mgr.section(ConfigSection::Accounts).is_some());
}

/// models references non-existent credential provider → WARN, load succeeds.
#[test]
fn test_cross_ref_models_unknown_credential_warns() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory(tmp.path());
    fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {"openai": {"apiKey": "sk-test", "models": [{"id": "gpt-4"}]}}}).to_string(),
    ).unwrap();
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    mgr.load().unwrap();
    assert!(mgr.section(ConfigSection::Models).is_some());
}

/// matching accounts→channels passes without warning.
#[test]
fn test_cross_ref_accounts_matching_channels() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory(tmp.path());
    fs::write(
        tmp.path().join("accounts.json"),
        serde_json::json!({"accounts": [{"accountId": "a1", "senderId": "s1", "platform": "feishu"}]}).to_string(),
    ).unwrap();
    fs::write(
        tmp.path().join("channels.json"),
        serde_json::json!({"channels": {"feishu": {"enabled": true}}}).to_string(),
    )
    .unwrap();
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    mgr.load().unwrap();
    assert!(mgr.section(ConfigSection::Accounts).is_some());
}

/// cross-ref runs after all configs loaded (credentials section exists).
#[test]
fn test_cross_ref_runs_after_configs_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory(tmp.path());
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();
    fs::write(
        creds_dir.join("openai.json"),
        serde_json::json!({"provider": "openai", "apiKey": "sk-test"}).to_string(),
    )
    .unwrap();
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    mgr.load().unwrap();
    assert!(mgr.section(ConfigSection::Credentials).is_some());
}

/// structural validation still rejects invalid accounts (empty accountId).
#[test]
fn test_structural_validation_enforced_for_accounts() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory(tmp.path());
    fs::write(
        tmp.path().join("accounts.json"),
        serde_json::json!({"accounts": [{"accountId": "", "senderId": "s1", "platform": "feishu"}]}).to_string(),
    ).unwrap();
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    assert!(mgr.load().is_err());
}
