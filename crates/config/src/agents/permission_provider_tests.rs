use super::*;
use filetime::FileTime;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

fn make_perms(agent_id: &str) -> AgentPermissions {
    let mut permissions = HashMap::new();
    permissions.insert(
        "file_read".to_string(),
        ActionPermission {
            allowed: true,
            limits: Default::default(),
        },
    );
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

#[test]
fn test_get_returns_permissions_when_file_exists() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("test-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let perms = make_perms("test-agent");
    let json = serde_json::to_string(&perms).unwrap();
    fs::write(agent_dir.join("permissions.json"), json).unwrap();

    let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
    let result = provider.get("test-agent");
    assert!(result.is_some());
    let loaded = result.unwrap();
    assert_eq!(loaded.agent_id, "test-agent");
    assert!(loaded.is_allowed("file_read"));
}

#[test]
fn test_get_returns_none_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
    assert!(provider.get("nonexistent").is_none());
}

#[test]
fn test_cache_hit_on_unchanged_mtime() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("cached-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let perms = make_perms("cached-agent");
    let json = serde_json::to_string(&perms).unwrap();
    fs::write(agent_dir.join("permissions.json"), &json).unwrap();

    let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());

    // First call loads from disk.
    let first = provider.get("cached-agent").unwrap();
    assert_eq!(first.agent_id, "cached-agent");

    // Second call should use cache (same mtime).
    let second = provider.get("cached-agent").unwrap();
    assert_eq!(second.agent_id, "cached-agent");
}

#[test]
fn test_cache_invalidation_on_file_replacement() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("mut-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let path = agent_dir.join("permissions.json");

    let perms1 = make_perms("mut-agent");
    fs::write(&path, serde_json::to_string(&perms1).unwrap()).unwrap();

    let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
    let first = provider.get("mut-agent").unwrap();
    assert!(first.is_allowed("file_read"));

    // Rewrite the file with different content and explicitly advance the
    // mtime so the cache is reliably invalidated even on fast machines.
    let mut perms2 = make_perms("mut-agent");
    perms2.permissions.insert(
        "file_read".to_string(),
        ActionPermission {
            allowed: false,
            limits: Default::default(),
        },
    );
    fs::write(&path, serde_json::to_string(&perms2).unwrap()).unwrap();
    // Advance mtime by 10 seconds to guarantee cache invalidation.
    let now = FileTime::now();
    let new_mtime = FileTime::from_unix_time(now.unix_seconds() + 10, now.nanoseconds());
    filetime::set_file_mtime(&path, new_mtime).unwrap();

    let second = provider.get("mut-agent").unwrap();
    assert!(!second.is_allowed("file_read"));
}

#[test]
fn test_json_parse_error_returns_none() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("bad-json");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("permissions.json"), "not valid json {{{").unwrap();

    let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
    assert!(provider.get("bad-json").is_none());
}

// =========================================================================
// Two-level permissions loading tests (project > user priority)
// =========================================================================

/// Helper: create a permissions file with a specific `file_read` state.
fn write_perms(dir: &std::path::Path, agent_id: &str, allowed: bool) {
    let agent_dir = dir.join(agent_id);
    fs::create_dir_all(&agent_dir).unwrap();
    let mut permissions = HashMap::new();
    permissions.insert(
        "file_read".to_string(),
        ActionPermission {
            allowed,
            limits: Default::default(),
        },
    );
    let perms = AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    };
    fs::write(
        agent_dir.join("permissions.json"),
        serde_json::to_string(&perms).unwrap(),
    )
    .unwrap();
}

/// Only project-level exists → use project-level rules.
#[test]
fn test_project_level_only_uses_project_rules() {
    let tmp = TempDir::new().unwrap();
    let user_agents = tmp.path().join("user_agents");
    let project_agents = tmp.path().join("project_agents");

    // Project-level: file_read allowed
    write_perms(&project_agents, "alpha", true);

    let provider = LazyAgentPermissions::new(user_agents.clone());
    provider.set_project_agents_dir(project_agents.clone());

    let result = provider.get("alpha").unwrap();
    assert!(result.is_allowed("file_read"));
}

/// Only user-level exists → behavior unchanged (user-level rules).
#[test]
fn test_user_level_only_uses_user_rules() {
    let tmp = TempDir::new().unwrap();
    let user_agents = tmp.path().join("user_agents");
    let project_agents = tmp.path().join("project_agents");

    // User-level: file_read allowed — write under agents/ subdirectory
    // to match user_permissions_path = config_dir/agents/<id>/permissions.json
    let user_agents_dir = user_agents.join("agents");
    write_perms(&user_agents_dir, "beta", true);

    let provider = LazyAgentPermissions::new(user_agents.clone());
    provider.set_project_agents_dir(project_agents.clone());

    let result = provider.get("beta").unwrap();
    assert!(result.is_allowed("file_read"));
}

/// Both levels exist → project-level takes priority (overrides user-level).
#[test]
fn test_both_levels_project_takes_priority() {
    let tmp = TempDir::new().unwrap();
    let user_agents = tmp.path().join("user_agents");
    let project_agents = tmp.path().join("project_agents");

    // User-level: file_read allowed
    write_perms(&user_agents, "gamma", true);
    // Project-level: file_read denied
    write_perms(&project_agents, "gamma", false);

    let provider = LazyAgentPermissions::new(user_agents.clone());
    provider.set_project_agents_dir(project_agents.clone());

    let result = provider.get("gamma").unwrap();
    // Project-level should win: file_read is denied.
    assert!(!result.is_allowed("file_read"));
}

/// Neither level exists → system default baseline (None).
#[test]
fn test_neither_level_returns_none() {
    let tmp = TempDir::new().unwrap();
    let user_agents = tmp.path().join("user_agents");
    let project_agents = tmp.path().join("project_agents");

    let provider = LazyAgentPermissions::new(user_agents.clone());
    provider.set_project_agents_dir(project_agents.clone());

    assert!(provider.get("nonexistent").is_none());
}

/// Cache invalidation: after project file changes, stale cache is refreshed.
#[test]
fn test_project_level_cache_invalidation() {
    let tmp = TempDir::new().unwrap();
    let user_agents = tmp.path().join("user_agents");
    let project_agents = tmp.path().join("project_agents");

    // Initial: project-level file_read allowed
    write_perms(&project_agents, "delta", true);

    let provider = LazyAgentPermissions::new(user_agents.clone());
    provider.set_project_agents_dir(project_agents.clone());

    let first = provider.get("delta").unwrap();
    assert!(first.is_allowed("file_read"));

    // Update project-level: file_read denied
    write_perms(&project_agents, "delta", false);
    let now = FileTime::now();
    let new_mtime = FileTime::from_unix_time(now.unix_seconds() + 10, now.nanoseconds());
    filetime::set_file_mtime(
        project_agents.join("delta").join("permissions.json"),
        new_mtime,
    )
    .unwrap();

    let second = provider.get("delta").unwrap();
    assert!(!second.is_allowed("file_read"));
}
