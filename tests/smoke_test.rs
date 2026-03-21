//! Integration smoke tests - verify all modules work together

use closeclaw::skills::{SkillRegistry, builtin_skills};
use closeclaw::agent::{Agent, AgentState};
use closeclaw::permission::PermissionEngine;
use closeclaw::config::ConfigProvider;

#[tokio::test]
async fn test_skill_registry_with_builtins() {
    let registry = SkillRegistry::new();
    
    // Register built-in skills
    for skill in builtin_skills() {
        registry.register(skill).await;
    }
    
    // Verify skills are registered
    let skills = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
}

#[tokio::test]
async fn test_agent_creation() {
    let agent = Agent::new("test-agent".to_string(), None);
    assert_eq!(agent.state, AgentState::Idle);
    assert!(agent.is_alive(300)); // 5 minute timeout
}

#[tokio::test]
async fn test_permission_engine_parse() {
    let json = r#"{
        "version": "1.0",
        "rules": [],
        "defaults": { "effect": "deny" }
    }"#;
    
    let _rules: closeclaw::permission::RuleSet = serde_json::from_str(json).unwrap();
}

#[test]
fn test_config_provider_trait_exists() {
    // Verify ConfigProvider trait is accessible
    fn _check<T: ConfigProvider>() {}
}
