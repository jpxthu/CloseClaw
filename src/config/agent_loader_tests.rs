//! Unit tests for `agent_loader.rs` — agent registration list loading
//! and ID union merge logic.
//!
//! Covers:
//! - `load_agents_json`: parsing user/project agents.json
//! - `merge_agent_ids`: union of two ID lists
//! - `load_agents`: project-level agents.json loading
//!
//! Tests use tempdirs to simulate user and project config layouts.
//!
//! `ConfigManager` is created with `config_dir = <tmp>/config/`.
//! `load_agents_json` reads `<config_dir>/agents.json`, so
//! agents.json goes into `<tmp>/config/agents.json`.
//! `load_agents` derives `user_agents_dir = <config_dir>/agents/`.

use std::fs;
use std::path::Path;

use crate::config::ConfigManager;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write an agents.json file with the given IDs.
///
/// `dir` is the directory to write agents.json into (should be the
/// config directory, i.e. `~/.closeclaw/`).
fn write_agents_json(dir: &Path, ids: &[&str]) {
    let content = serde_json::json!({ "agents": ids });
    let json_str = serde_json::to_string_pretty(&content).unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("agents.json"), json_str).unwrap();
}

/// Create a minimal `config.json` for an agent so
/// `AgentDirectoryProvider` can resolve it.
fn create_agent_config(agents_dir: &Path, id: &str) {
    let agent_dir = agents_dir.join(id);
    fs::create_dir_all(&agent_dir).unwrap();
    let config = serde_json::json!({
        "id": id,
        "name": id,
    });
    fs::write(
        agent_dir.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
}

/// Build a `ConfigManager` with `config_dir = <base>/config/`.
/// The agents directory is `<base>/agents/` (root level).
fn make_config_manager(base: &Path) -> ConfigManager {
    let config_dir = base.join("config");
    fs::create_dir_all(&config_dir).unwrap();
    ConfigManager::new(config_dir).expect("ConfigManager::new should succeed")
}

// ===========================================================================
// merge_agent_ids tests
// ===========================================================================

/// Verify that `merge_agent_ids` produces a union of two lists
/// with first-wins deduplication.
#[test]
fn test_merge_agent_ids_union() {
    let user = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
    let project = vec!["beta".to_string(), "delta".to_string(), "alpha".to_string()];

    let merged = ConfigManager::merge_agent_ids(&user, &project);

    assert_eq!(merged.len(), 4, "should have 4 unique IDs");
    assert_eq!(merged[0], "alpha");
    assert_eq!(merged[1], "beta");
    assert_eq!(merged[2], "gamma");
    assert_eq!(merged[3], "delta");
}

/// When project list is empty, merged result equals user list.
#[test]
fn test_merge_agent_ids_empty_project() {
    let user = vec!["a".to_string(), "b".to_string()];
    let project: Vec<String> = vec![];
    let merged = ConfigManager::merge_agent_ids(&user, &project);
    assert_eq!(merged, vec!["a", "b"]);
}

/// When both lists are empty, result is empty.
#[test]
fn test_merge_agent_ids_both_empty() {
    let user: Vec<String> = vec![];
    let project: Vec<String> = vec![];
    let merged = ConfigManager::merge_agent_ids(&user, &project);
    assert!(merged.is_empty());
}

// ===========================================================================
// load_agents_json tests
// ===========================================================================

/// Loading a non-existent agents.json should return an empty list.
#[test]
fn test_load_agents_json_nonexistent() {
    let tmp = tempfile::tempdir().unwrap();
    let cm = make_config_manager(tmp.path());
    let result = cm.load_agents_json(&tmp.path().join("nonexistent.json"));
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

/// Loading a valid agents.json should parse the IDs correctly.
#[test]
fn test_load_agents_json_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    write_agents_json(&config_dir, &["orchestrator", "coder"]);
    let cm = make_config_manager(tmp.path());
    let ids = cm
        .load_agents_json(&config_dir.join("agents.json"))
        .unwrap();
    assert_eq!(ids, vec!["orchestrator", "coder"]);
}

/// Loading a JSONC agents.json with comments should strip comments
/// and parse correctly.
#[test]
fn test_load_agents_json_with_comments() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    // Comments come first, then real entries (avoids trailing comma)
    let json = r#"{
  "agents": [
    // "commented-out-agent",
    "a",
    "c"
  ]
}"#;
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("agents.json"), json).unwrap();
    let cm = make_config_manager(tmp.path());
    let ids = cm
        .load_agents_json(&config_dir.join("agents.json"))
        .unwrap();
    assert_eq!(ids, vec!["a", "c"]);
}

// ===========================================================================
// load_agents — project-level agents.json integration
// ===========================================================================

/// When both user and project agents.json exist, their IDs should
/// be merged (union).
#[test]
fn test_load_agents_merges_user_and_project() {
    let tmp = tempfile::tempdir().unwrap();
    let cm = make_config_manager(tmp.path());

    // agents.json goes into config_dir, agents go into root/agents
    let config_dir = tmp.path().join("config");
    let root_dir = tmp.path();
    let agents_dir = root_dir.join("agents");

    write_agents_json(&config_dir, &["agent-1", "agent-2"]);
    create_agent_config(&agents_dir, "agent-1");
    create_agent_config(&agents_dir, "agent-2");

    // Fake repo root with project-level agents.json
    let repo_dir = tempfile::tempdir().unwrap();
    let project_closeclaw = repo_dir.path().join(".closeclaw");
    fs::create_dir_all(&project_closeclaw).unwrap();
    write_agents_json(&project_closeclaw, &["agent-2", "agent-3"]);
    let project_agents = project_closeclaw.join("agents");
    create_agent_config(&project_agents, "agent-2");
    create_agent_config(&project_agents, "agent-3");

    cm.load_agents(Some(repo_dir.path()))
        .expect("load_agents should succeed");

    let agents = cm.agents();
    let mut ids: Vec<&str> = agents.keys().map(|s| s.as_str()).collect();
    ids.sort();
    assert!(
        ids.contains(&"agent-1"),
        "user-level agent-1 should be present"
    );
    assert!(ids.contains(&"agent-2"), "agent-2 should be present");
    assert!(
        ids.contains(&"agent-3"),
        "project-level agent-3 should be present"
    );
    assert_eq!(ids.len(), 3, "should have exactly 3 unique agents");
}

/// When repo_root is None, only user-level agents.json is loaded.
#[test]
fn test_load_agents_no_project_root() {
    let tmp = tempfile::tempdir().unwrap();
    let cm = make_config_manager(tmp.path());
    let config_dir = tmp.path().join("config");
    let root_dir = tmp.path();
    let agents_dir = root_dir.join("agents");

    write_agents_json(&config_dir, &["user-agent"]);
    create_agent_config(&agents_dir, "user-agent");

    cm.load_agents(None).expect("load_agents should succeed");

    let agents = cm.agents();
    let ids: Vec<&str> = agents.keys().map(|s| s.as_str()).collect();
    assert_eq!(ids, vec!["user-agent"]);
}

/// When project-level agents.json does not exist, only user-level
/// is loaded (no error).
#[test]
fn test_load_agents_project_json_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let cm = make_config_manager(tmp.path());
    let config_dir = tmp.path().join("config");
    let root_dir = tmp.path();
    let agents_dir = root_dir.join("agents");

    write_agents_json(&config_dir, &["only-user"]);
    create_agent_config(&agents_dir, "only-user");

    let repo_dir = tempfile::tempdir().unwrap();

    cm.load_agents(Some(repo_dir.path()))
        .expect("load_agents should succeed even without project json");

    let agents = cm.agents();
    let ids: Vec<&str> = agents.keys().map(|s| s.as_str()).collect();
    assert_eq!(ids, vec!["only-user"]);
}

/// Reload should re-read agents from disk.
#[test]
fn test_reload_agents_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    let cm = make_config_manager(tmp.path());
    let config_dir = tmp.path().join("config");
    let root_dir = tmp.path();
    let agents_dir = root_dir.join("agents");

    write_agents_json(&config_dir, &["a"]);
    create_agent_config(&agents_dir, "a");

    cm.load_agents(None).unwrap();
    assert!(cm.agents().contains_key("a"));

    // Add a new agent
    write_agents_json(&config_dir, &["a", "b"]);
    create_agent_config(&agents_dir, "b");
    cm.reload_agents().unwrap();

    let agents = cm.agents();
    let mut ids: Vec<&str> = agents.keys().map(|s| s.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a", "b"]);
}
