//! Tests for daemon config hot-reload module.

use super::*;

#[test]
fn test_filename_to_section() {
    assert_eq!(
        filename_to_section("models.json"),
        Some(ConfigSection::Models)
    );
    assert_eq!(
        filename_to_section("channels.json"),
        Some(ConfigSection::Channels)
    );
    assert_eq!(
        filename_to_section("gateway.json"),
        Some(ConfigSection::Gateway)
    );
    assert_eq!(
        filename_to_section("plugins.json"),
        Some(ConfigSection::Plugins)
    );
    assert_eq!(
        filename_to_section("system.json"),
        Some(ConfigSection::System)
    );
    // agents.json is NOT mapped — handled via reload_agents() separately
    assert_eq!(filename_to_section("agents.json"), None);
    assert_eq!(filename_to_section("unknown.json"), None);
}

/// Replicate the agents_dir path construction used by both
/// `setup_watcher()` and `init_config_hot_reload()` so the logic
/// can be tested without filesystem side-effects.
fn agents_dir_for_config(config_dir: &str) -> std::path::PathBuf {
    let config_path = std::path::Path::new(config_dir);
    config_path.join("agents")
}

#[test]
fn test_agents_dir_normal_config() {
    let result = agents_dir_for_config("~/.closeclaw/config");
    assert_eq!(
        result,
        std::path::PathBuf::from("~/.closeclaw/config/agents")
    );
}

#[test]
fn test_agents_dir_root_config() {
    let result = agents_dir_for_config("/config");
    assert_eq!(result, std::path::PathBuf::from("/config/agents"));
}

#[test]
fn test_agents_dir_fallback_no_parent() {
    // When config_dir is "/" (root), agents go into "/agents".
    let result = agents_dir_for_config("/");
    assert_eq!(result, std::path::PathBuf::from("/agents"));
}
