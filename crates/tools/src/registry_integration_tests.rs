//! Integration tests for SkillTool and SkillCreatorTool registration.
//!
//! These tests exercise real tool implementations (not `DummyTool`) to verify
//! they appear correctly in the tool index.  Migrated from `registry_tests.rs`
//! to keep that file under the 1 000-line limit.

use super::*;
use std::sync::Arc;

/// SkillTool (eager) and SkillCreatorTool (deferred) appear in separate
/// groups in the first-level index. SkillTool's group shows "(always loaded)"
/// and SkillCreatorTool's group shows "(deferred)".
#[tokio::test]
async fn test_index_builder_skill_tool_eager_skill_creator_deferred() {
    use closeclaw_skills::disk::DiskSkillRegistry;
    use closeclaw_skills::BuiltinSkillRegistry;

    let reg = ToolRegistry::new();
    // SkillTool — is_deferred_by_default = false (Step 1.1)
    let skill_tool = crate::builtin::skill_tool::SkillTool::new(
        Arc::new(DiskSkillRegistry::new(vec![])),
        Arc::new(BuiltinSkillRegistry::new()),
    );
    reg.register(skill_tool).await.unwrap();
    // SkillCreatorTool — is_deferred_by_default = true (Step 1.2)
    let creator_tool = closeclaw_skills::SkillCreatorTool::new();
    reg.register(creator_tool).await.unwrap();

    let ctx = make_prompt_ctx(&["SkillTool", "SkillCreator"]);
    let section = reg.build_tools_section(&ctx).await;

    // skills group has SkillTool (eager) → "(always loaded)"
    assert!(
        section.contains("**skills** — (always loaded)"),
        "skills group should be always loaded, got: {section}"
    );
    // SkillTool appears with bold name + detail (eager)
    assert!(
        section.contains("**SkillTool**"),
        "SkillTool should be bold (eager), got: {section}"
    );
    // skill_creator group has only SkillCreatorTool (deferred) → "(deferred)"
    assert!(
        section.contains("**skill_creator** — (deferred)"),
        "skill_creator group should be deferred, got: {section}"
    );
    // SkillCreatorTool appears without bold (deferred)
    assert!(
        section.contains("  - SkillCreator"),
        "SkillCreator should appear without bold (deferred), got: {section}"
    );
    assert!(
        !section.contains("**SkillCreator**"),
        "SkillCreator should NOT be bold (deferred), got: {section}"
    );
}

/// SkillTool and SkillCreatorTool appear as registered tool names.
#[tokio::test]
async fn test_registry_has_skill_tool_and_skill_creator() {
    use closeclaw_skills::disk::DiskSkillRegistry;
    use closeclaw_skills::BuiltinSkillRegistry;

    let reg = ToolRegistry::new();
    let skill_tool = crate::builtin::skill_tool::SkillTool::new(
        Arc::new(DiskSkillRegistry::new(vec![])),
        Arc::new(BuiltinSkillRegistry::new()),
    );
    reg.register(skill_tool).await.unwrap();
    let creator_tool = closeclaw_skills::SkillCreatorTool::new();
    reg.register(creator_tool).await.unwrap();

    let names = reg.list_tool_names().await;
    assert!(names.contains(&"SkillTool".to_string()));
    assert!(names.contains(&"SkillCreator".to_string()));
}
