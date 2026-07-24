//! Tests for built-in skills
use crate::builtin::{builtin_skills, BuiltinSkills, FileOpsSkill, GitOpsSkill, SearchSkill};
use crate::registry::Skill;
use closeclaw_common::permission_types::{
    CallerInfo, RiskLevel, SharedSkillApprovalSubmitter, SharedSkillPermissionChecker,
};
use std::sync::Arc;

/// Mock permission checker that always allows.
struct AllowAllChecker;

#[async_trait::async_trait]
impl closeclaw_common::permission_types::SkillPermissionChecker for AllowAllChecker {
    async fn check_permission(
        &self,
        _action: &str,
        _resource: &str,
        _details: serde_json::Value,
    ) -> closeclaw_common::permission_types::PermissionEvalResult {
        closeclaw_common::permission_types::PermissionEvalResult::Allowed {
            context_modifier: None,
        }
    }
}

/// Mock permission checker that always denies.
struct DenyAllChecker;

#[async_trait::async_trait]
impl closeclaw_common::permission_types::SkillPermissionChecker for DenyAllChecker {
    async fn check_permission(
        &self,
        _action: &str,
        _resource: &str,
        _details: serde_json::Value,
    ) -> closeclaw_common::permission_types::PermissionEvalResult {
        closeclaw_common::permission_types::PermissionEvalResult::Denied {
            reason: "access denied by mock".to_string(),
            risk_level: RiskLevel::Low,
        }
    }
}

#[tokio::test]
async fn test_file_ops_read_requires_agent_id_when_engine_set() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("read", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_ok());

    let checker = Arc::new(AllowAllChecker)
        as Arc<dyn closeclaw_common::permission_types::SkillPermissionChecker>;
    let skill_with = FileOpsSkill::with_engine(checker);
    let result = skill_with
        .execute("read", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_file_ops_read_with_permission() {
    let checker = Arc::new(AllowAllChecker)
        as Arc<dyn closeclaw_common::permission_types::SkillPermissionChecker>;
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "read should succeed with permission: {:?}",
        result
    );
    let value = result.unwrap();
    assert!(value.get("content").is_some());
}

#[tokio::test]
async fn test_file_ops_read_denied_without_permission() {
    let checker = Arc::new(DenyAllChecker)
        as Arc<dyn closeclaw_common::permission_types::SkillPermissionChecker>;
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::registry::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_file_ops_exists_requires_agent_id_when_engine_set() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("exists", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_ok());

    let checker = Arc::new(AllowAllChecker)
        as Arc<dyn closeclaw_common::permission_types::SkillPermissionChecker>;
    let skill_with = FileOpsSkill::with_engine(checker);
    let result = skill_with
        .execute("exists", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_file_ops_exists_with_permission() {
    let checker = Arc::new(AllowAllChecker)
        as Arc<dyn closeclaw_common::permission_types::SkillPermissionChecker>;
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "exists",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "exists should succeed with permission: {:?}",
        result
    );
}

#[tokio::test]
async fn test_git_ops_status() {
    let skill = GitOpsSkill::new();
    let result = skill.execute("status", serde_json::json!({})).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_search() {
    let skill = SearchSkill::new();
    let result = skill
        .execute("search", serde_json::json!({"query": "rust programming"}))
        .await;
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("is_stub").and_then(|v| v.as_bool()) == Some(true));
}

#[test]
fn test_builtin_skills() {
    let skills = BuiltinSkills::all();
    assert_eq!(skills.len(), 7);
    assert_eq!(skills[0].manifest().name, "file_ops");
    assert_eq!(skills[1].manifest().name, "git_ops");
    assert_eq!(skills[2].manifest().name, "search");
    assert_eq!(skills[3].manifest().name, "permission_query");
    assert_eq!(skills[4].manifest().name, "skill_discovery");
    assert_eq!(skills[5].manifest().name, "coding_agent");
    assert_eq!(skills[6].manifest().name, "skill_creator");
}

#[tokio::test]
async fn test_skill_registry_with_builtins() {
    use crate::registry::BuiltinSkillRegistry;
    let registry = BuiltinSkillRegistry::new();
    for skill in builtin_skills() {
        registry.register(skill).await;
    }
    let skills: Vec<String> = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
}

// ── Mock approval submitter ──────────────────────────────────────────────

struct MockApprovalSubmitter {
    last_submitted: std::sync::Mutex<Option<String>>,
}

impl MockApprovalSubmitter {
    fn new() -> Self {
        Self {
            last_submitted: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl closeclaw_common::permission_types::SkillApprovalSubmitter for MockApprovalSubmitter {
    async fn submit_denial(
        &self,
        action: &str,
        resource: &str,
        _reason: &str,
        _risk_level: RiskLevel,
        _session_id: &str,
        _caller: &CallerInfo,
    ) -> Option<String> {
        let id = format!("req-{}-{}", action, resource);
        *self.last_submitted.lock().unwrap() = Some(id.clone());
        Some(id)
    }
}

// Mock checker that denies a specific action.
struct DenySpecificChecker {
    deny_action: String,
}

#[async_trait::async_trait]
impl closeclaw_common::permission_types::SkillPermissionChecker for DenySpecificChecker {
    async fn check_permission(
        &self,
        action: &str,
        _resource: &str,
        _details: serde_json::Value,
    ) -> closeclaw_common::permission_types::PermissionEvalResult {
        if action == self.deny_action {
            closeclaw_common::permission_types::PermissionEvalResult::Denied {
                reason: "specific deny".to_string(),
                risk_level: RiskLevel::Medium,
            }
        } else {
            closeclaw_common::permission_types::PermissionEvalResult::Allowed {
                context_modifier: None,
            }
        }
    }
}

// ── Permission denied tests (mock checker) ──────────────────────────────

#[tokio::test]
async fn test_install_permission_denied() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "spawn".to_string(),
    });
    let skill = crate::builtin::SkillDiscoverySkill::with_engine(checker);
    let result = skill
        .execute(
            "install",
            serde_json::json!({"agent_id": "a1", "skill": "foo"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::registry::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_install_permission_with_extra_deny() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "spawn".to_string(),
    });
    let approval: SharedSkillApprovalSubmitter = Arc::new(MockApprovalSubmitter::new());
    let skill =
        crate::builtin::SkillDiscoverySkill::with_engine_and_approval_flow(checker, approval);
    let result = skill
        .execute(
            "install",
            serde_json::json!({"agent_id": "a1", "skill": "foo"}),
        )
        .await;
    // With approval flow, denied request should return approval_pending
    assert!(result.is_ok());
    let v = result.unwrap();
    assert_eq!(v["status"], "approval_pending");
}

#[tokio::test]
async fn test_file_ops_read_permission_denied() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "file_read".to_string(),
    });
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": "/tmp/x", "agent_id": "a1"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::registry::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_file_ops_list_permission_denied() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "file_read".to_string(),
    });
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "list",
            serde_json::json!({"path": "/tmp", "agent_id": "a1"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::registry::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_file_ops_delete_permission_denied() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "file_write".to_string(),
    });
    let skill = FileOpsSkill::with_engine(checker);
    let result = skill
        .execute(
            "delete",
            serde_json::json!({"path": "/tmp/x", "agent_id": "a1"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::registry::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_file_ops_write_permission_denied_with_approval() {
    let checker: SharedSkillPermissionChecker = Arc::new(DenySpecificChecker {
        deny_action: "file_write".to_string(),
    });
    let approval: SharedSkillApprovalSubmitter = Arc::new(MockApprovalSubmitter::new());
    let skill = FileOpsSkill::with_engine_and_approval_flow(checker, approval);
    let result = skill
        .execute(
            "write",
            serde_json::json!({"path": "/tmp/x", "content": "data", "agent_id": "a1"}),
        )
        .await;
    assert!(result.is_ok());
    let v = result.unwrap();
    assert_eq!(v["status"], "approval_pending");
}

#[tokio::test]
async fn test_file_ops_read_allowed_passes_through() {
    let checker: SharedSkillPermissionChecker = Arc::new(AllowAllChecker);
    let skill = FileOpsSkill::with_engine(checker);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "hello").unwrap();
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": path.to_str().unwrap(), "agent_id": "a1"}),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap()["content"], "hello");
}

#[tokio::test]
async fn test_file_ops_list_allowed() {
    let checker: SharedSkillPermissionChecker = Arc::new(AllowAllChecker);
    let skill = FileOpsSkill::with_engine(checker);
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let result = skill
        .execute(
            "list",
            serde_json::json!({"path": dir.path().to_str().unwrap(), "agent_id": "a1"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "list should succeed with allowed permission: {:?}",
        result
    );
    let binding = result.unwrap();
    let entries = binding["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn test_file_ops_delete_allowed() {
    let checker: SharedSkillPermissionChecker = Arc::new(AllowAllChecker);
    let skill = FileOpsSkill::with_engine(checker);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("to_delete.txt");
    std::fs::write(&path, "bye").unwrap();
    let result = skill
        .execute(
            "delete",
            serde_json::json!({"path": path.to_str().unwrap(), "agent_id": "a1"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "delete should succeed with allowed permission: {:?}",
        result
    );
    assert!(result.unwrap()["success"] == true);
    assert!(!path.exists(), "file should be deleted");
}
