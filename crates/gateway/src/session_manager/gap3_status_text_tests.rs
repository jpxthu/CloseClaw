//! Tests for Step 1.5 — status-dependent notification text (Gap 3).
//!
//! Validates that `drain_and_inject_announces` renders different
//! status labels depending on the child session's `ChildCompletionStatus`:
//! - `Completed` → "任务已完成"
//! - `Errored` → "任务出错"
//! - `Terminated` → "任务被终止"

use super::test_helpers::setup_parent_with_conv;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::session_manager::spawn::SpawnMode;
use crate::session_manager::test_helpers::register_child_only;
use crate::Session;
use chrono::Utc;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession, ConversationSession};
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

// ── helper ──────────────────────────────────────────────────────────────

fn make_event(
    child_id: &str,
    agent_id: &str,
    result_text: &str,
    status: ChildCompletionStatus,
) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: child_id.to_string(),
        child_agent_id: agent_id.to_string(),
        result_text: result_text.to_string(),
        completed_at: Utc::now(),
        priority: NotificationPriority::Next,
        status,
    }
}

#[allow(dead_code)]
async fn register_child_with_session(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    mode: SpawnMode,
) {
    register_child_only(mgr, parent_id, child_id, agent_id, mode).await;

    // Also register in parent's child_states so update_child_state works.
    {
        let parent_cs = mgr.get_conversation_session(parent_id).await.unwrap();
        let guard = parent_cs.read().await;
        guard.child_states.write().expect("lock").insert(
            child_id.to_string(),
            (closeclaw_common::ChildSessionState::Running, None),
        );
    }

    let cs = Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    )));
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), cs);
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        Session {
            id: child_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "feishu".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 1,
        },
    );
}

// ── 1. Completed → "任务已完成" ──────────────────────────────────────────

/// When a child session completes successfully, the injected system
/// message must contain "任务已完成" status label.
#[tokio::test]
#[serial]
async fn test_status_text_completed() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-sc").await;

    let event = make_event(
        "child-sc",
        "worker-sc",
        "result text here",
        ChildCompletionStatus::Completed,
    );
    mgr.push_announce(&parent_id, event).await.unwrap();

    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 1, "expected one injected message");
    assert_eq!(msgs[0].role, "system");

    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        text.contains("任务已完成"),
        "Completed status must produce '任务已完成' label, got: {}",
        text
    );
    assert!(
        text.contains("worker-sc"),
        "text should contain agent id, got: {}",
        text
    );
    assert!(
        text.contains("result text here"),
        "text should contain result, got: {}",
        text
    );
}

// ── 2. Errored → "任务出错" ─────────────────────────────────────────────

/// When a child session errors, the injected system message must
/// contain "任务出错" status label.
#[tokio::test]
#[serial]
async fn test_status_text_errored() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-se").await;

    let event = make_event(
        "child-se",
        "worker-se",
        "error occurred",
        ChildCompletionStatus::Errored,
    );
    mgr.push_announce(&parent_id, event).await.unwrap();

    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 1);

    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        text.contains("任务出错"),
        "Errored status must produce '任务出错' label, got: {}",
        text
    );
}

// ── 3. Terminated → "任务被终止" ────────────────────────────────────────

/// When a child session is terminated, the injected system message must
/// contain "任务被终止" status label.
#[tokio::test]
#[serial]
async fn test_status_text_terminated() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-st").await;

    let event = make_event(
        "child-st",
        "worker-st",
        "killed by user",
        ChildCompletionStatus::Terminated,
    );
    mgr.push_announce(&parent_id, event).await.unwrap();

    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 1);

    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        text.contains("任务被终止"),
        "Terminated status must produce '任务被终止' label, got: {}",
        text
    );
}

// ── 4. Mixed statuses in a single drain ─────────────────────────────────

/// When multiple events with different statuses are queued, each must
/// render its own status label.
#[tokio::test]
#[serial]
async fn test_status_text_mixed_statuses() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-sxm").await;

    mgr.push_announce(
        &parent_id,
        make_event("c1", "a1", "done", ChildCompletionStatus::Completed),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &parent_id,
        make_event("c2", "a2", "oops", ChildCompletionStatus::Errored),
    )
    .await
    .unwrap();
    mgr.push_announce(
        &parent_id,
        make_event("c3", "a3", "killed", ChildCompletionStatus::Terminated),
    )
    .await
    .unwrap();

    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 3, "expected 3 injected messages");

    let texts: Vec<String> = msgs
        .iter()
        .map(|m| match &m.content_blocks[0] {
            ContentBlock::Text(t) => t.clone(),
            other => panic!("expected Text, got {:?}", other),
        })
        .collect();

    assert!(
        texts.iter().any(|t| t.contains("任务已完成")),
        "should contain Completed label"
    );
    assert!(
        texts.iter().any(|t| t.contains("任务出错")),
        "should contain Errored label"
    );
    assert!(
        texts.iter().any(|t| t.contains("任务被终止")),
        "should contain Terminated label"
    );
}

// ── 5. No status label confusion ────────────────────────────────────────

/// Verify that Completed events do not accidentally include labels
/// from other statuses, and vice versa.
#[tokio::test]
#[serial]
async fn test_status_text_no_cross_contamination() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-sx-no").await;

    mgr.push_announce(
        &parent_id,
        make_event("c1", "a1", "ok", ChildCompletionStatus::Completed),
    )
    .await
    .unwrap();

    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text, got {:?}", other),
    };
    assert!(
        !text.contains("任务出错"),
        "Completed event must not contain Errored label, got: {}",
        text
    );
    assert!(
        !text.contains("任务被终止"),
        "Completed event must not contain Terminated label, got: {}",
        text
    );
}

// ── 6. Status text via drain_and_inject with Errored event ─────────────

/// Directly push an Errored event and verify the injected text
/// contains "任务出错". This exercises the same code path that
/// `try_push_announce` uses, without the complexity of full
/// session registration.
#[tokio::test]
#[serial]
async fn test_status_text_errored_via_direct_push() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-rp-e").await;

    let event = make_event(
        "child-rp-e",
        "worker-rp-e",
        "error output",
        ChildCompletionStatus::Errored,
    );
    mgr.push_announce(&parent_id, event).await.unwrap();

    // Verify drain returns the event with Errored status.
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event");
    assert_eq!(
        drained[0].status,
        ChildCompletionStatus::Errored,
        "status should be Errored"
    );

    // Push it back and inject via drain_and_inject.
    mgr.push_announce(&parent_id, drained.into_iter().next().unwrap())
        .await
        .unwrap();
    mgr.drain_and_inject_announces(&parent_id).await;

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 1);
    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text, got {:?}", other),
    };
    assert!(
        text.contains("任务出错"),
        "Errored status should produce '任务出错' label, got: {}",
        text
    );
}
