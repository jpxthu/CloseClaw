use super::{skill, MockAgentSkillsQuery};
use crate::disk::types::{ScanConfig, SkillSource};
use crate::disk::DiskSkillRegistry;
use closeclaw_common::AgentSkillsQuery;
use std::sync::Arc;

#[test]
fn test_replace_skills_swaps_list() {
    let mut reg = DiskSkillRegistry::new(vec![skill("old", SkillSource::Bundled)]);
    assert_eq!(reg.len(), 1);
    assert!(reg.contains("old"));

    reg.replace_skills(vec![
        skill("new-a", SkillSource::Global),
        skill("new-b", SkillSource::Agent),
    ]);

    assert_eq!(reg.len(), 2);
    assert!(!reg.contains("old"));
    assert!(reg.contains("new-a"));
    assert!(reg.contains("new-b"));
}

#[test]
fn test_replace_skills_preserves_agent_skills_query() {
    let mut reg = DiskSkillRegistry::new(vec![]);
    let query: Arc<dyn AgentSkillsQuery> =
        Arc::new(MockAgentSkillsQuery::new().with_config("a", vec![]));
    reg.set_agent_skills_query(Arc::clone(&query));
    assert!(reg.agent_skills_query().is_some());

    reg.replace_skills(vec![skill("x", SkillSource::Bundled)]);

    assert!(reg.agent_skills_query().is_some());
    let returned = reg.agent_skills_query().unwrap();
    assert!(Arc::ptr_eq(returned, &query));
}

#[test]
fn test_replace_skills_empty_vec_clears_registry() {
    let mut reg = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
    ]);
    assert_eq!(reg.len(), 2);

    reg.replace_skills(vec![]);

    assert_eq!(reg.len(), 0);
    assert!(reg.is_empty());
}

#[test]
fn test_scan_config_returns_none_when_unset() {
    let reg = DiskSkillRegistry::new(vec![]);
    assert!(reg.scan_config().is_none());
}

#[test]
fn test_scan_config_returns_config_when_set() {
    let mut reg = DiskSkillRegistry::new(vec![]);
    let config = ScanConfig {
        global_dir: Some(std::path::PathBuf::from("/tmp/global")),
        extra_dirs: vec![std::path::PathBuf::from("/tmp/extra")],
        ..Default::default()
    };
    reg.set_scan_config(config);

    let returned = reg.scan_config().unwrap();
    assert_eq!(
        returned.global_dir,
        Some(std::path::PathBuf::from("/tmp/global"))
    );
    assert_eq!(
        returned.extra_dirs,
        vec![std::path::PathBuf::from("/tmp/extra")]
    );
}
