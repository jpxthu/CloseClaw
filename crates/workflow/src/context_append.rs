//! Workflow context injection helpers.
//!
//! Provides [`build_workflow_context_append`] for constructing the
//! system_appends workflow context string, used by:
//!
//! - Step 1.3: `/workflow <name>` slash handler (initial injection)
//! - Step 1.4: Post-compaction re-injection
//! - Step 1.5: Session recovery re-injection
//!
//! Format matches the design doc spec (`session-integration.md` §system prompt 注入).

use crate::definition::Workflow;

/// Build the workflow context string to inject into system_appends.
///
/// Returns a string in the format:
///
/// ```text
/// --- WORKFLOW ---
/// 你正在执行受控工作流：{workflow_name}
/// 描述：{description}
/// Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：
/// 1. 收到 goal → 执行步骤
/// 2. 收到 verify → 自查验收清单 → 完成则调用 workflow_verify，否则继续
/// 3. 收到 jump → 回答问题 → 调用 workflow_jump 传递答案
/// 不要自行跳步或跳过验证。
/// --- WORKFLOW END ---
/// ```
pub fn build_workflow_context_append(workflow: &Workflow) -> String {
    format!(
        "--- WORKFLOW ---\n\
         你正在执行受控工作流：{name}\n\
         描述：{desc}\n\
         Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：\n\
         1. 收到 goal → 执行步骤\n\
         2. 收到 verify → 自查验收清单 → 完成则调用 workflow_verify，否则继续\n\
         3. 收到 jump → 回答问题 → 调用 workflow_jump 传递答案\n\
         不要自行跳步或跳过验证。\n\
         --- WORKFLOW END ---",
        name = workflow.name,
        desc = workflow.description,
    )
}

/// Check whether a workflow context marker exists in the given
/// system_appends list.
///
/// Returns `true` if any item starts with `"--- WORKFLOW ---"`.
pub fn has_workflow_context(system_appends: &[String]) -> bool {
    system_appends
        .iter()
        .any(|s| s.starts_with("--- WORKFLOW ---"))
}

/// Remove all workflow context markers from a system_appends list.
///
/// Removes items that start with `"--- WORKFLOW ---"`. Returns the
/// count of items removed.
pub fn remove_workflow_context(system_appends: &mut Vec<String>) -> usize {
    let before = system_appends.len();
    system_appends.retain(|s| !s.starts_with("--- WORKFLOW ---"));
    before - system_appends.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{Step, Workflow};

    fn make_test_workflow() -> Workflow {
        Workflow {
            id: "test-wf".to_string(),
            name: "Test Workflow".to_string(),
            description: "A test workflow".to_string(),
            version: Some("0.1".to_string()),
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps: vec![Step {
                id: 0,
                name: "Step Zero".to_string(),
                allow_blocked: None,
                goal: "Do the first thing".to_string(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
            }],
        }
    }

    #[test]
    fn test_build_workflow_context_append_format() {
        let wf = make_test_workflow();
        let ctx = build_workflow_context_append(&wf);
        assert!(ctx.starts_with("--- WORKFLOW ---"));
        assert!(ctx.ends_with("--- WORKFLOW END ---"));
        assert!(ctx.contains("Test Workflow"));
        assert!(ctx.contains("A test workflow"));
        assert!(ctx.contains("workflow_verify"));
        assert!(ctx.contains("workflow_jump"));
        assert!(ctx.contains("三阶段协议"));
    }

    #[test]
    fn test_has_workflow_context_present() {
        let appends = vec![
            "some other append".to_string(),
            build_workflow_context_append(&make_test_workflow()),
        ];
        assert!(has_workflow_context(&appends));
    }

    #[test]
    fn test_has_workflow_context_absent() {
        let appends = vec!["some append".to_string(), "another append".to_string()];
        assert!(!has_workflow_context(&appends));
    }

    #[test]
    fn test_has_workflow_context_empty() {
        let appends: Vec<String> = vec![];
        assert!(!has_workflow_context(&appends));
    }

    #[test]
    fn test_remove_workflow_context() {
        let mut appends = vec![
            "before".to_string(),
            build_workflow_context_append(&make_test_workflow()),
            "after".to_string(),
        ];
        let removed = remove_workflow_context(&mut appends);
        assert_eq!(removed, 1);
        assert_eq!(appends, vec!["before".to_string(), "after".to_string()]);
    }

    #[test]
    fn test_remove_workflow_context_none() {
        let mut appends = vec!["no workflow here".to_string()];
        let removed = remove_workflow_context(&mut appends);
        assert_eq!(removed, 0);
        assert_eq!(appends, vec!["no workflow here".to_string()]);
    }

    /// Simulate post-compaction re-injection: workflow context is
    /// cleared during compaction, then re-injected from checkpoint.
    #[test]
    fn test_compaction_re_injection() {
        let wf = make_test_workflow();
        let context = build_workflow_context_append(&wf);

        // Simulate post-compaction state: system_appends without
        // workflow context (compaction may clear it).
        let mut appends = vec![
            "existing system append".to_string(),
            // workflow context was cleared by compaction
        ];

        // Verify context is missing.
        assert!(!has_workflow_context(&appends));

        // Re-inject workflow context (as done in
        // reinject_workflow_context_after_compact).
        appends.push(context.clone());

        // Verify context is present.
        assert!(has_workflow_context(&appends));
        assert_eq!(appends.len(), 2);
        assert_eq!(appends[1], context);
    }

    /// Verify that re-injection is idempotent: if workflow context
    /// already exists, re-injection should not duplicate it.
    #[test]
    fn test_compaction_re_injection_idempotent() {
        let wf = make_test_workflow();
        let context = build_workflow_context_append(&wf);

        // System_appends already has workflow context.
        let mut appends = vec!["existing system append".to_string(), context.clone()];

        // Verify context is present.
        assert!(has_workflow_context(&appends));

        // Re-injection should not duplicate (caller checks
        // has_workflow_context before injecting).
        if !has_workflow_context(&appends) {
            appends.push(context.clone());
        }

        // Verify no duplication.
        assert_eq!(appends.len(), 2);
        assert_eq!(appends[1], context);
    }
}
