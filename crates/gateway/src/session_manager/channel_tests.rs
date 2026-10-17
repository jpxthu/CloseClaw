//! Tests for `channel::force_new_for_channel` workdir fallback branches.

use super::tests::make_test_mgr;
use super::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

/// `workspace_dir` 缺失时 `/new` 新建会话的 workdir 回退系统临时目录。
#[tokio::test]
async fn test_force_new_workspace_none_falls_back_to_temp_dir() {
    let expected = std::env::temp_dir();
    let mgr = make_test_mgr(None);

    let session_id = mgr.force_new_for_channel("feishu", "agent-ws-none").await;

    let conv = mgr
        .get_conversation_session(&session_id)
        .await
        .expect("ConversationSession should exist");
    assert_eq!(
        conv.read().await.workdir(),
        expected.as_path(),
        "missing workspace_dir must fall back to temp dir"
    );
}

/// ensure_workspace_dir 失败（agent_id 含 `/`，纯验证层报错）→ 静默回退
/// 系统临时目录，会话仍创建、不 panic、不向上传播错误。
#[tokio::test]
async fn test_force_new_ensure_failed_falls_back_to_temp_dir() {
    let expected = std::env::temp_dir();
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = SessionManager::new(
        &super::tests::test_config(),
        None,
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );

    let session_id = mgr.force_new_for_channel("feishu", "bad/agent").await;

    let conv = mgr
        .get_conversation_session(&session_id)
        .await
        .expect("ConversationSession should exist");
    assert_eq!(
        conv.read().await.workdir(),
        expected.as_path(),
        "ensure failure must silently fall back to temp dir"
    );
}
