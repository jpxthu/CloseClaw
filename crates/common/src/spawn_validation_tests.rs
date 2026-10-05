//! Behavior tests for the spawn-validation contract after the doc alignment
//! (`config: ResolvedAgentConfig` → `agent_id: String`,
//! `PermissionDenied { .. }` → `Permission(SpawnPermissionError)`).
//!
//! sessions_spawn 工具侧的权限拒绝行为（两步校验顺序、拒绝 reason 原样
//! 透传、拒绝后不创建子会话）由
//! `crates/session/src/tools/sessions_spawn_tests.rs` 覆盖，此处不重复；
//! 本文件验证共享载荷本身的契约：`agent_id` 形态的值语义与
//! `SpawnError::Permission` 的匹配路径 / 消息透传。

use super::*;
use crate::SpawnPermissionError;

fn sample_validation() -> SpawnValidationResult {
    SpawnValidationResult {
        agent_id: "child-agent".to_string(),
        effective_max_spawn_depth: 2,
        spawn_timeout: Some(900),
        timeout_warning_secs: Some(60),
        timeout_notify_interval_ratio: Some(0.5),
    }
}

/// `SpawnValidationResult` is identified by `agent_id` (the target agent's
/// full config never enters the shared structure) and has value semantics:
/// cloning deep-copies the identifier instead of aliasing it.
#[test]
fn test_spawn_validation_result_agent_id_construction_and_clone() {
    let result = sample_validation();
    assert_eq!(result.agent_id, "child-agent");
    assert_eq!(result.effective_max_spawn_depth, 2);
    assert_eq!(result.spawn_timeout, Some(900));
    assert_eq!(result.timeout_warning_secs, Some(60));
    assert_eq!(result.timeout_notify_interval_ratio, Some(0.5));

    let cloned = result.clone();
    assert_eq!(cloned.agent_id, result.agent_id);
    assert_eq!(
        cloned.effective_max_spawn_depth,
        result.effective_max_spawn_depth
    );
    assert_eq!(cloned.spawn_timeout, result.spawn_timeout);
    assert_eq!(cloned.timeout_warning_secs, result.timeout_warning_secs);
    assert_eq!(
        cloned.timeout_notify_interval_ratio,
        result.timeout_notify_interval_ratio
    );

    let mut cloned = cloned;
    cloned.agent_id = "other-agent".to_string();
    cloned.spawn_timeout = None;
    assert_eq!(
        result.agent_id, "child-agent",
        "clone must not alias the original agent_id"
    );
    assert_eq!(
        result.spawn_timeout,
        Some(900),
        "clone must not alias the original spawn_timeout"
    );
}

/// `SpawnError::Permission` reuses `SpawnPermissionError` as payload: the
/// `Denied { agent_id, reason }` fields stay matchable through the nested
/// variant, and the rendered message keeps the pre-migration denial wording
/// verbatim (`#[error(transparent)]` — no wrapper prefix is added).
#[test]
fn test_spawn_error_permission_matches_denied_payload_and_message() {
    let inner = SpawnPermissionError::Denied {
        agent_id: "child-agent".to_string(),
        reason: "not in parent allowlist".to_string(),
    };
    let err = SpawnError::Permission(inner.clone());

    match &err {
        SpawnError::Permission(SpawnPermissionError::Denied { agent_id, reason }) => {
            assert_eq!(agent_id, "child-agent");
            assert_eq!(reason, "not in parent allowlist");
        }
        other => panic!("expected SpawnError::Permission(Denied), got {other:?}"),
    }

    assert_eq!(
        err.to_string(),
        "spawn permission denied for agent 'child-agent': not in parent allowlist",
        "permission denial message must keep its pre-migration wording"
    );
    assert_eq!(
        err.to_string(),
        inner.to_string(),
        "SpawnError::Permission must not decorate the inner denial message"
    );
}
