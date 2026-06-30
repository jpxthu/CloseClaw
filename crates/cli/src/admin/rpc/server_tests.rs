use std::sync::Arc;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_skills::DiskSkillRegistry;

use crate::admin::rpc::protocol::{AdminRequest, AdminResponse};
use crate::admin::rpc::server::{
    dispatch, dispatch_agent_create, dispatch_agent_info, dispatch_agent_list,
    dispatch_skill_install, dispatch_skill_list, AdminContext,
};

fn make_test_context() -> AdminContext {
    let config_dir = tempfile::tempdir().unwrap().keep();
    let config_sub = config_dir.join("config");
    std::fs::create_dir_all(&config_sub).unwrap();
    // agents.json lives in the config subdirectory
    // (ConfigManager.config_dir = config_sub)
    std::fs::write(config_sub.join("agents.json"), r#"{"agents": []}"#).unwrap();
    let config_manager = Arc::new(closeclaw_config::ConfigManager::new(config_sub).unwrap());
    AdminContext {
        agent_registry: Arc::new(AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(DiskSkillRegistry::default()))),
        config_manager,
        config_dir,
    }
}

#[test]
fn test_dispatch_agent_list_empty() {
    let ctx = make_test_context();
    let resp = dispatch_agent_list(&ctx);
    match resp {
        AdminResponse::AgentListResult { agents } => assert!(agents.is_empty()),
        _ => panic!("expected AgentListResult"),
    }
}

#[test]
fn test_dispatch_agent_info_not_found() {
    let ctx = make_test_context();
    let resp = dispatch_agent_info("nonexistent", &ctx);
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn test_dispatch_skill_list_empty() {
    let ctx = make_test_context();
    let resp = dispatch_skill_list(&ctx).await;
    match resp {
        AdminResponse::SkillListResult { skills } => assert!(skills.is_empty()),
        _ => panic!("expected SkillListResult"),
    }
}

#[tokio::test]
async fn test_dispatch_skill_install_not_found() {
    let ctx = make_test_context();
    let resp = dispatch_skill_install("test-skill", &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error for missing skill"),
    }
}

#[tokio::test]
async fn test_dispatch_agent_create_empty_name() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("", Some("gpt-4".into()), &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("empty"));
        }
        _ => panic!("expected Error for empty name"),
    }
}

#[tokio::test]
async fn test_dispatch_agent_create_success() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("test-agent", Some("gpt-4".into()), &ctx).await;
    assert!(matches!(resp, AdminResponse::Ok));
    // Verify agent was created
    assert!(ctx.agent_registry.get("test-agent").is_some());
}

#[tokio::test]
async fn test_dispatch_agent_create_duplicate() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
    assert!(matches!(resp, AdminResponse::Ok));
    // Try creating same agent again
    let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("already exists"));
        }
        _ => panic!("expected Error for duplicate"),
    }
}

#[tokio::test]
async fn test_dispatch_ping() {
    let ctx = make_test_context();
    let resp = dispatch(AdminRequest::Ping, &ctx).await;
    assert!(matches!(resp, AdminResponse::Pong));
}
