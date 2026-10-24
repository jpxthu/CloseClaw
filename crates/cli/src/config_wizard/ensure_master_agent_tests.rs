use super::*;
use closeclaw_common::test_helpers::ensure_dir;
use tempfile::TempDir;

fn isolated_dirs(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_dir = tmp.path().join("config");
    let agents_dir = tmp.path().join("agents");
    ensure_dir(&config_dir).unwrap();
    ensure_dir(&agents_dir).unwrap();
    (config_dir, agents_dir)
}

fn assert_master_config(config_path: &std::path::Path) {
    assert!(config_path.exists(), "master config.json should exist");
    let content = std::fs::read_to_string(config_path).unwrap();
    let config: AgentConfig =
        serde_json::from_str(&content).expect("master config.json should be valid JSON");
    assert_eq!(config.id, "master");
    assert!(config.name.is_none());
    assert!(config.parent_id.is_none());
    assert!(config.model.is_none());
    assert!(config.workspace.is_none());
    assert!(config.agent_dir.is_none());
    assert_eq!(
        config.bootstrap_mode,
        Some(closeclaw_common::BootstrapMode::Full)
    );
    assert_eq!(config.skills, vec!["*"]);
    assert_eq!(config.tools, vec!["*"]);
    assert!(config.disallowed_tools.is_empty());
}

fn assert_agents_json(agents_json_path: &std::path::Path, expected: &[&str]) {
    assert!(agents_json_path.exists(), "agents.json should exist");
    let content = std::fs::read_to_string(agents_json_path).unwrap();
    let config: AgentsConfig =
        serde_json::from_str(&content).expect("agents.json should be valid JSON");
    let expected_owned: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(config.agents, expected_owned);
}

/// Test 1: empty directory → both files created with correct content.
#[test]
fn test_creates_on_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, agents_dir) = isolated_dirs(&tmp);
    ensure_master_agent(&config_dir, &agents_dir).unwrap();

    let master_config_path = agents_dir.join("master").join("config.json");
    let agents_json_path = config_dir.join("agents.json");

    assert_master_config(&master_config_path);
    assert_agents_json(&agents_json_path, &["master"]);
}

/// Test 2: idempotent — calling twice does not modify existing files.
#[test]
fn test_idempotent() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, agents_dir) = isolated_dirs(&tmp);
    ensure_master_agent(&config_dir, &agents_dir).unwrap();

    let master_config_path = agents_dir.join("master").join("config.json");
    let agents_json_path = config_dir.join("agents.json");

    let config_before = std::fs::read_to_string(&master_config_path).unwrap();
    let agents_before = std::fs::read_to_string(&agents_json_path).unwrap();

    // Second call should not change anything
    ensure_master_agent(&config_dir, &agents_dir).unwrap();

    let config_after = std::fs::read_to_string(&master_config_path).unwrap();
    let agents_after = std::fs::read_to_string(&agents_json_path).unwrap();

    assert_eq!(
        config_before, config_after,
        "config.json should not be modified"
    );
    assert_eq!(
        agents_before, agents_after,
        "agents.json should not be modified"
    );
}

/// Test 3: agents.json already has other agents → master is appended.
#[test]
fn test_appends_master_to_existing_agents_json() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, agents_dir) = isolated_dirs(&tmp);

    let agents_json_path = config_dir.join("agents.json");
    std::fs::write(
        &agents_json_path,
        serde_json::to_string_pretty(&AgentsConfig {
            agents: vec!["helper".to_string(), "researcher".to_string()],
        })
        .unwrap(),
    )
    .unwrap();

    ensure_master_agent(&config_dir, &agents_dir).unwrap();

    // agents.json should now contain helper, researcher, master
    assert_agents_json(&agents_json_path, &["helper", "researcher", "master"]);

    // master config.json should also exist
    let master_config_path = agents_dir.join("master").join("config.json");
    assert_master_config(&master_config_path);
}

/// Test 4: config.json exists but agents.json missing master → master appended.
#[test]
fn test_agents_json_missing_master_only() {
    let tmp = TempDir::new().unwrap();
    let (config_dir, agents_dir) = isolated_dirs(&tmp);

    // Pre-create master config.json (already exists)
    ensure_dir(&agents_dir.join("master")).unwrap();
    let existing_config = AgentConfig {
        id: "master".to_string(),
        ..Default::default()
    };
    std::fs::write(
        agents_dir.join("master").join("config.json"),
        serde_json::to_string_pretty(&existing_config).unwrap(),
    )
    .unwrap();

    // Pre-create agents.json WITHOUT master
    let agents_json_path = config_dir.join("agents.json");
    std::fs::write(
        &agents_json_path,
        serde_json::to_string_pretty(&AgentsConfig {
            agents: vec!["helper".to_string()],
        })
        .unwrap(),
    )
    .unwrap();

    ensure_master_agent(&config_dir, &agents_dir).unwrap();

    // config.json should remain unchanged (not overwritten)
    let config_content =
        std::fs::read_to_string(agents_dir.join("master").join("config.json")).unwrap();
    let config: AgentConfig = serde_json::from_str(&config_content).unwrap();
    assert_eq!(config.id, "master");

    // agents.json should now include master
    assert_agents_json(&agents_json_path, &["helper", "master"]);
}
