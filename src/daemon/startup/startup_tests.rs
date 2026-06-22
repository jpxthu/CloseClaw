use super::*;

#[test]
fn test_component_id_name() {
    assert_eq!(ComponentId::ConfigManager.name(), "ConfigManager");
    assert_eq!(ComponentId::Gateway.name(), "Gateway");
    assert_eq!(ComponentId::DreamingScheduler.name(), "DreamingScheduler");
}
