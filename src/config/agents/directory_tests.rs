//! Tests for `AgentDirectoryProvider`.

use super::*;
use crate::agent::config::{ActionPermission, PermissionLimits};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Write a minimal `config.json` for the given agent ID.
fn write_config(dir: &Path, id: &str, name: &str) {
    let agent_dir = dir.join(id);
    std::fs::create_dir_all(&agent_dir).unwrap();
    let json = format!(r#"{{ "id": "{}", "name": "{}" }}"#, id, name);
    std::fs::write(agent_dir.join("config.json"), json).unwrap();
}

/// Write a minimal `permissions.json` for the given agent ID.
fn write_permissions(dir: &Path, id: &str, marker: &str) {
    let agent_dir = dir.join(id);
    std::fs::create_dir_all(&agent_dir).unwrap();
    // Embed `marker` inside the agent_id so we can assert which file won.
    let json = format!(
        r#"{{ "agent_id": "{}",
"permissions": {{ "exec": {{ "allowed": true,
"limits": {{}} }} }} }}"#,
        marker
    );
    std::fs::write(agent_dir.join("permissions.json"), json).unwrap();
}

#[test]
fn test_empty_registry_produces_no_entries() {
    let user = TempDir::new().unwrap();
    // Create a stray agent dir that must NOT be picked up.
    write_config(user.path(), "stray", "Stray Agent");

    let provider =
        AgentDirectoryProvider::new(Vec::new(), user.path().to_path_buf(), None).unwrap();

    assert!(provider.agent_ids().is_empty());
    assert!(provider.entries().is_empty());
    assert!(provider.permissions().is_empty());
}

#[test]
fn test_user_only_load() {
    let user = TempDir::new().unwrap();
    write_config(user.path(), "alpha", "Alpha Agent");

    let provider =
        AgentDirectoryProvider::new(vec!["alpha".to_string()], user.path().to_path_buf(), None)
            .unwrap();

    assert_eq!(provider.agent_ids().len(), 1);
    let entry = provider.get("alpha").expect("alpha should be loaded");
    assert_eq!(entry.id, "alpha");
    assert_eq!(entry.name, "Alpha Agent");
    assert_eq!(entry.source, ConfigSource::User);
}

#[test]
fn test_project_only_load() {
    let project = TempDir::new().unwrap();
    write_config(project.path(), "beta", "Beta Agent");

    let provider = AgentDirectoryProvider::new(
        vec!["beta".to_string()],
        PathBuf::from("/nonexistent/user/agents"),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    assert_eq!(provider.agent_ids().len(), 1);
    let entry = provider.get("beta").expect("beta should be loaded");
    assert_eq!(entry.id, "beta");
    assert_eq!(entry.name, "Beta Agent");
    assert_eq!(entry.source, ConfigSource::Project);
}

#[test]
fn test_merge_project_overrides_user() {
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    write_config(user.path(), "gamma", "User Name");
    write_config(project.path(), "gamma", "Project Name");

    let provider = AgentDirectoryProvider::new(
        vec!["gamma".to_string()],
        user.path().to_path_buf(),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    let entry = provider.get("gamma").expect("gamma should be loaded");
    // Project name wins.
    assert_eq!(entry.name, "Project Name");
    assert_eq!(entry.source, ConfigSource::Merged);
}

#[test]
fn test_ignores_dirs_outside_registry() {
    let user = TempDir::new().unwrap();
    // Files in the registry → must be loaded.
    write_config(user.path(), "registered", "Registered");
    // Files NOT in the registry → must be ignored.
    write_config(user.path(), "unregistered", "Unregistered");

    let provider = AgentDirectoryProvider::new(
        vec!["registered".to_string()],
        user.path().to_path_buf(),
        None,
    )
    .unwrap();

    assert_eq!(provider.agent_ids(), vec![&"registered".to_string()]);
    assert!(provider.get("unregistered").is_none());
}

#[test]
fn test_permissions_project_wins_over_user() {
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    write_config(user.path(), "delta", "Delta");
    write_config(project.path(), "delta", "Delta");
    write_permissions(user.path(), "delta", "user-marker");
    write_permissions(project.path(), "delta", "project-marker");

    let provider = AgentDirectoryProvider::new(
        vec!["delta".to_string()],
        user.path().to_path_buf(),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    let perms = provider
        .permissions()
        .get("delta")
        .expect("delta permissions should be loaded");
    // Project permissions win → agent_id carries the project marker.
    assert_eq!(perms.agent_id, "project-marker");
}

#[test]
fn test_permissions_user_fallback_when_no_project_file() {
    let user = TempDir::new().unwrap();
    write_config(user.path(), "epsilon", "Epsilon");
    write_permissions(user.path(), "epsilon", "user-marker");

    let provider = AgentDirectoryProvider::new(
        vec!["epsilon".to_string()],
        user.path().to_path_buf(),
        Some(PathBuf::from("/nonexistent/project/agents")),
    )
    .unwrap();

    let perms = provider
        .permissions()
        .get("epsilon")
        .expect("epsilon permissions should be loaded from user");
    assert_eq!(perms.agent_id, "user-marker");
    assert!(perms.is_allowed("exec"));
}

#[test]
fn test_missing_config_json_is_skipped() {
    let user = TempDir::new().unwrap();
    // Create the agent directory but leave it empty (no config.json).
    std::fs::create_dir_all(user.path().join("zeta")).unwrap();
    std::fs::write(user.path().join("zeta/.placeholder"), b"").unwrap();
    // Another agent that DOES have a config file.
    write_config(user.path(), "eta", "Eta");

    let provider = AgentDirectoryProvider::new(
        vec!["zeta".to_string(), "eta".to_string()],
        user.path().to_path_buf(),
        None,
    )
    .unwrap();

    // zeta has no config.json → skipped.
    assert!(provider.get("zeta").is_none());
    // eta is still loaded.
    assert!(provider.get("eta").is_some());
    assert_eq!(provider.agent_ids().len(), 1);
}

#[test]
fn test_reload_picks_up_changes() {
    let user = TempDir::new().unwrap();
    let provider =
        AgentDirectoryProvider::new(vec!["theta".to_string()], user.path().to_path_buf(), None)
            .unwrap();
    assert!(provider.get("theta").is_none());

    // Add a config file and reload.
    write_config(user.path(), "theta", "Theta");
    let provider = provider;
    // Provider has no public `reload` callable here? Yes it does.
    // We need `&mut self`, so reconstruct via the constructor for the
    // first call, then mutate via `reload` after the change.
    // Easier: use the constructor twice.
    drop(provider);

    let provider =
        AgentDirectoryProvider::new(vec!["theta".to_string()], user.path().to_path_buf(), None)
            .unwrap();
    assert!(provider.get("theta").is_some());
}

#[test]
fn test_no_user_dir_no_project_dir() {
    // Neither user nor project dir exists. The registry IDs should all
    // be skipped, and no errors should be raised.
    let provider = AgentDirectoryProvider::new(
        vec!["a".to_string(), "b".to_string()],
        PathBuf::from("/nonexistent/user"),
        None,
    )
    .unwrap();
    assert!(provider.agent_ids().is_empty());
    assert!(provider.entries().is_empty());
}

#[test]
fn test_merge_falls_back_to_user_field_when_project_empty() {
    // When the user config sets a field the project config does not,
    // the user value must be preserved.
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    // user: name and a skill; project: only a different name.
    let user_dir = user.path().join("iota");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(
        user_dir.join("config.json"),
        r#"{ "id": "iota", "name": "Iota", "skills": ["web"] }"#,
    )
    .unwrap();

    let project_dir = project.path().join("iota");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(
        project_dir.join("config.json"),
        r#"{ "id": "iota", "name": "Iota Project" }"#,
    )
    .unwrap();

    let provider = AgentDirectoryProvider::new(
        vec!["iota".to_string()],
        user.path().to_path_buf(),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    let entry = provider.get("iota").expect("iota must be loaded");
    // Project name overrides user name.
    assert_eq!(entry.name, "Iota Project");
    // Project-level default skills=["*"] overrides user-level skills.
    assert_eq!(entry.skills, vec!["*".to_string()]);
    assert_eq!(entry.source, ConfigSource::Merged);
}

#[test]
fn test_no_permissions_file_is_fine() {
    let user = TempDir::new().unwrap();
    write_config(user.path(), "kappa", "Kappa");

    let provider =
        AgentDirectoryProvider::new(vec!["kappa".to_string()], user.path().to_path_buf(), None)
            .unwrap();

    assert!(provider.get("kappa").is_some());
    assert!(provider.permissions().get("kappa").is_none());
}

#[test]
fn test_action_permission_round_trip() {
    // Sanity check: ActionPermission is the type used inside
    // AgentPermissions; this guards against accidental type changes.
    let perm = ActionPermission {
        allowed: true,
        limits: PermissionLimits::default(),
    };
    let json = serde_json::to_string(&perm).unwrap();
    let back: ActionPermission = serde_json::from_str(&json).unwrap();
    assert!(back.allowed);
}

// =====================================================================
// Step 1.5 — Tests for `AgentDirectoryProvider` dirname-id backfill and
// WARN-on-mismatch behaviour.
// =====================================================================

/// `config.json` without an `id` field must fail to load — the agent
/// is skipped (deserialization of the required `id` field fails).
#[test]
fn test_directory_provider_id_from_dirname() {
    let user = TempDir::new().unwrap();
    // Directory name is `foo`; the config.json omits `id` entirely.
    std::fs::create_dir_all(user.path().join("foo")).unwrap();
    std::fs::write(
        user.path().join("foo/config.json"),
        r#"{ "name": "Foo Agent" }"#,
    )
    .unwrap();

    // Create a valid agent to ensure loading continues after the skip.
    write_config(user.path(), "bar", "Bar Agent");

    let provider = AgentDirectoryProvider::new(
        vec!["foo".to_string(), "bar".to_string()],
        user.path().to_path_buf(),
        None,
    )
    .unwrap();

    // foo has no `id` → deserialization fails → agent is skipped.
    assert!(provider.get("foo").is_none());
    // bar is still loaded normally.
    let entry = provider.get("bar").expect("bar should be loaded");
    assert_eq!(entry.id, "bar");
    assert_eq!(entry.name, "Bar Agent");
}

/// A `config.json` `id` that disagrees with the directory name must
/// produce a WARN log; the config's id is kept as-is.
#[test]
fn test_directory_provider_id_mismatch_warn() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// A `MakeWriter` that clones an `Arc<Mutex<Vec<u8>>>` buffer so
    /// the subscriber can write into it while the test still owns the
    /// original handle to read the captured bytes back.
    #[derive(Clone, Default)]
    struct VecWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for VecWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for VecWriter {
        type Writer = VecWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    let buffer = VecWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_target(false)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let user = TempDir::new().unwrap();
    // Directory name is `foo`, but the config.json declares id `other`.
    std::fs::create_dir_all(user.path().join("foo")).unwrap();
    std::fs::write(
        user.path().join("foo/config.json"),
        r#"{ "id": "other", "name": "Other Agent" }"#,
    )
    .unwrap();

    let provider =
        AgentDirectoryProvider::new(vec!["foo".to_string()], user.path().to_path_buf(), None)
            .unwrap();

    // The config's id wins (the WARN message says so explicitly).
    let entry = provider.get("foo").expect("foo should be loaded");
    assert_eq!(entry.id, "other");
    assert_eq!(entry.name, "Other Agent");

    // Drop the subscriber guard so the captured buffer is fully flushed
    // before we read it.
    drop(_guard);
    let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("does not match directory name"),
        "expected WARN log, got: {}",
        output
    );
}

/// `config.json` with `id` set to an empty string `""` must cause
/// provider construction to fail — the empty id does not satisfy the
/// required-field constraint, so `new()` returns an error.
#[test]
fn test_directory_provider_empty_string_id_fails_construction() {
    let user = TempDir::new().unwrap();
    let agent_dir = user.path().join("empty-id");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("config.json"),
        r#"{ "id": "", "name": "Empty ID Agent" }"#,
    )
    .unwrap();

    let result = AgentDirectoryProvider::new(
        vec!["empty-id".to_string()],
        user.path().to_path_buf(),
        None,
    );

    assert!(
        result.is_err(),
        "empty-string id should cause provider to fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing required agent id"),
        "error should mention missing id, got: {}",
        err_msg
    );
}

// =====================================================================
// Step 1.4 — Tests for `load_agent_config` parse error path
// =====================================================================

/// Agent with invalid JSON in user config.json → warn and skip.
#[test]
fn test_user_config_parse_error_skips_agent() {
    let user = TempDir::new().unwrap();
    let agent_dir = user.path().join("bad-user");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("config.json"), "not valid json {{").unwrap();

    // Also create a valid agent to ensure loading continues.
    write_config(user.path(), "good-user", "Good Agent");

    let provider = AgentDirectoryProvider::new(
        vec!["bad-user".to_string(), "good-user".to_string()],
        user.path().to_path_buf(),
        None,
    )
    .unwrap();

    assert!(
        provider.get("bad-user").is_none(),
        "agent with invalid config.json should be skipped"
    );
    assert!(
        provider.get("good-user").is_some(),
        "valid agent should still be loaded"
    );
}

/// Agent with invalid JSON in project config.json → warn and skip project
/// config, but still load user config.
#[test]
fn test_project_config_parse_error_falls_back_to_user() {
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    write_config(user.path(), "mixed", "User Name");

    let project_dir = project.path().join("mixed");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("config.json"), "totally broken json").unwrap();

    let provider = AgentDirectoryProvider::new(
        vec!["mixed".to_string()],
        user.path().to_path_buf(),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    let entry = provider
        .get("mixed")
        .expect("agent should load from user config");
    assert_eq!(entry.name, "User Name");
    assert_eq!(entry.source, ConfigSource::User);
}

/// Both user and project config.json have invalid JSON → agent skipped entirely.
#[test]
fn test_both_configs_parse_error_skips_agent() {
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    let user_dir = user.path().join("both-bad");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("config.json"), "bad user json").unwrap();

    let project_dir = project.path().join("both-bad");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("config.json"), "bad project json").unwrap();

    let provider = AgentDirectoryProvider::new(
        vec!["both-bad".to_string()],
        user.path().to_path_buf(),
        Some(project.path().to_path_buf()),
    )
    .unwrap();

    assert!(
        provider.get("both-bad").is_none(),
        "agent with both configs broken should be skipped"
    );
}

/// Agent directory exists but config.json is a directory (not a file) →
/// read_to_string fails → agent is skipped.
#[test]
fn test_config_json_is_directory_skips_agent() {
    let user = TempDir::new().unwrap();
    let agent_dir = user.path().join("dir-agent");
    // Create config.json as a directory instead of a file
    std::fs::create_dir_all(agent_dir.join("config.json")).unwrap();

    write_config(user.path(), "normal", "Normal Agent");

    let provider = AgentDirectoryProvider::new(
        vec!["dir-agent".to_string(), "normal".to_string()],
        user.path().to_path_buf(),
        None,
    )
    .unwrap();

    assert!(provider.get("dir-agent").is_none());
    assert!(provider.get("normal").is_some());
}
