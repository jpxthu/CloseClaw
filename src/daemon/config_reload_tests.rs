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
