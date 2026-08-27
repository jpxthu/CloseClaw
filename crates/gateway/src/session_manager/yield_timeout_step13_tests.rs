//! Step 1.3 tests for yield timeout warning notification content (Gap ①)
//! and per-child spawn timeout priority (Gap ② regression).
//!
//! Covers:
//! - Cyclic warning: 5 notification elements (设定预期时长、实际运行时长、
//!   硬超时与剩余、context window 已用/总容量、prompt+completion 用量)
//! - Legacy mode (timeout_warning_secs=None): "未设定" label
//! - No child sessions: no panic, graceful degradation
//! - No LLM calls (request_count=0): estimation fallback path
//! - Gap ② regression: per-child spawn timeout → Now priority

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_resolved_config};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

fn mgr() -> Arc<super::SessionManager> {
    Arc::new(make_test_mgr(None))
}

// ══════════════════════════════════════════════════════════════════════════
// Gap ①: Normal path — cyclic warning includes all 5 elements
// ══════════════════════════════════════════════════════════════════════════

/// Cyclic warning notification should contain:
/// 1. 设定预期执行时长 (timeout_warning_secs value)
/// 2. 实际运行时长 (per-child elapsed)
/// 3. 硬超时时间及剩余时间
/// 4. context window 已用/总容量 (xx / yy tokens)
/// 5. prompt + completion token 用量
#[tokio::test]
#[serial]
async fn test_cyclic_warning_notification_contains_five_elements() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-cw5").await;

    // Spawn a child so we have per-child data.
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-cw5", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Enter Waiting; start cyclic warning (overall=10s, warning at 1s).
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 10, Some(1), None)
        .await;

    // Wait for the first cyclic warning to fire (at T=1s).
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Drain the queue to collect the warning.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    // Find the warning entry.
    let warning_text = entries
        .iter()
        .filter_map(|e| match e {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _) => {
                if text.contains("超时预警") {
                    Some(text.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .next()
        .expect("should have a warning notification");

    // Item 1: 设定预期执行时长.
    assert!(
        warning_text.contains("设定预期执行时长: 1 秒"),
        "warning should contain timeout_warning_secs=1, got: {}",
        warning_text
    );

    // Item 2: 实际运行时长 (per-child from created_at).
    assert!(
        warning_text.contains("已运行"),
        "warning should contain per-child elapsed time, got: {}",
        warning_text
    );

    // Item 3: 硬超时时间及剩余时间.
    assert!(
        warning_text.contains("硬超时时间: 10 秒"),
        "warning should contain hard timeout value, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("剩余:"),
        "warning should contain remaining time, got: {}",
        warning_text
    );

    // Item 4: context window 已用/总容量 (xx / yy tokens).
    assert!(
        warning_text.contains("context window:"),
        "warning should contain context window usage, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("/"),
        "context window line should have 'used / total' format, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("tokens"),
        "context window should mention 'tokens', got: {}",
        warning_text
    );

    // Item 5: prompt + completion token usage.
    assert!(
        warning_text.contains("prompt="),
        "warning should contain prompt token count, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("completion="),
        "warning should contain completion token count, got: {}",
        warning_text
    );

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}

// ══════════════════════════════════════════════════════════════════════════
// Gap ①: Legacy mode — "未设定" label for timeout_warning_secs
// ══════════════════════════════════════════════════════════════════════════

/// Legacy mode (timeout_warning_secs=None) should show "未设定（legacy 模式）"
/// for the timeout_warning_secs line, while still including the other 4 items.
#[tokio::test]
#[serial]
async fn test_legacy_warning_shows_unset_label() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-lgu").await;

    // Spawn a child.
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-lgu", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Enter Waiting; start with overall=61s (legacy warning fires at T=1).
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 61, None, None)
        .await;

    // Wait for the single legacy warning to fire (at T=1s).
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Drain the queue.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    let warning_text = entries
        .iter()
        .filter_map(|e| match e {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _) => {
                if text.contains("超时预警") {
                    Some(text.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .next()
        .expect("should have a warning notification in legacy mode");

    // Item 1: "未设定（legacy 模式）".
    assert!(
        warning_text.contains("未设定（legacy 模式）"),
        "legacy warning should show '未设定（legacy 模式）', got: {}",
        warning_text
    );

    // Item 2: per-child elapsed.
    assert!(
        warning_text.contains("已运行"),
        "legacy warning should contain per-child elapsed time, got: {}",
        warning_text
    );

    // Item 3: hard timeout & remaining.
    assert!(
        warning_text.contains("硬超时时间: 61 秒"),
        "legacy warning should contain hard timeout value, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("剩余:"),
        "legacy warning should contain remaining time, got: {}",
        warning_text
    );

    // Item 4: context window.
    assert!(
        warning_text.contains("context window:"),
        "legacy warning should contain context window usage, got: {}",
        warning_text
    );

    // Item 5: token usage.
    assert!(
        warning_text.contains("prompt="),
        "legacy warning should contain prompt token count, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("completion="),
        "legacy warning should contain completion token count, got: {}",
        warning_text
    );

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}

// ══════════════════════════════════════════════════════════════════════════
// Gap ①: No child sessions — no panic, graceful degradation
// ══════════════════════════════════════════════════════════════════════════

/// When no child sessions exist, the warning notification should still
/// be injected without panicking, showing "(无子 session)".
#[tokio::test]
#[serial]
async fn test_cyclic_warning_no_children_no_panic() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-ncw").await;

    // No children spawned.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 5, Some(1), None)
        .await;

    // Wait for warning to fire.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Drain and verify no panic + correct content.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    let warning_text = entries
        .iter()
        .filter_map(|e| match e {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _) => {
                if text.contains("超时预警") {
                    Some(text.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .next()
        .expect("should have a warning notification even with no children");

    assert!(
        warning_text.contains("无子 session"),
        "warning with no children should show '(无子 session)', got: {}",
        warning_text
    );

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}

// ══════════════════════════════════════════════════════════════════════════
// Gap ①: No LLM calls (request_count=0) — estimation path still works
// ══════════════════════════════════════════════════════════════════════════

/// When a child session has no LLM calls (request_count=0), the
/// `estimate_total_tokens` function should fall back to pure character-based
/// estimation and still produce a valid "xx / yy tokens" output.
#[tokio::test]
#[serial]
async fn test_cyclic_warning_no_llm_calls_estimation_works() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-nlc").await;

    // Spawn a child but don't add any messages (request_count=0).
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-nlc", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 5, Some(1), None)
        .await;

    // Wait for warning to fire.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Drain and verify the estimation path produces valid output.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    let warning_text = entries
        .iter()
        .filter_map(|e| match e {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _) => {
                if text.contains("超时预警") {
                    Some(text.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .next()
        .expect("should have a warning notification");

    // Context window line should still be present and contain digits.
    let cw_line: Vec<&str> = warning_text
        .lines()
        .filter(|l| l.contains("context window:"))
        .collect();
    assert!(
        !cw_line.is_empty(),
        "should have a context window line even with no LLM calls, got: {}",
        warning_text
    );

    // Token usage should show 0/0 for a fresh session.
    assert!(
        warning_text.contains("prompt=0"),
        "prompt tokens should be 0 with no LLM calls, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("completion=0"),
        "completion tokens should be 0 with no LLM calls, got: {}",
        warning_text
    );

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}

// ══════════════════════════════════════════════════════════════════════════
// Gap ② regression: per-child spawn timeout → Now priority
// ══════════════════════════════════════════════════════════════════════════

/// When a per-child spawn timeout fires, the AnnounceEvent injected
/// into the parent's queue must carry `NotificationPriority::Now`,
/// not `Next`. This is a regression test for Gap ②.
#[tokio::test]
#[serial]
async fn test_per_child_spawn_timeout_uses_now_priority() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-nto").await;

    // Spawn a child with a very short per-child timeout (1 second).
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-nto", None),
            &parent_id,
            1,
            "quick task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            Some(1), // 1-second per-child timeout
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Wait for per-child timeout to fire (1s + buffer).
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Drain the parent's announce queue.
    let drained = m.drain_announces(&parent_id).await;

    // Find the spawn timeout event.
    let timeout_event = drained
        .iter()
        .find(|ev| ev.result_text.contains("spawn timeout"))
        .expect("should have a spawn timeout announce event");

    // Gap ② regression: must be Now priority.
    assert_eq!(
        timeout_event.priority,
        NotificationPriority::Now,
        "per-child spawn timeout must use Now priority (Gap ② regression), got: {:?}",
        timeout_event.priority
    );
}

// ══════════════════════════════════════════════════════════════════════════
// Cyclic warning: Next priority preserved (non-regression)
// ══════════════════════════════════════════════════════════════════════════

/// The yield warning notification itself should use Next priority
/// (not Now), as documented. This is a non-regression check.
#[tokio::test]
#[serial]
async fn test_cyclic_warning_uses_next_priority() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-cnp").await;

    // Spawn a child.
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-cnp", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 10, Some(1), None)
        .await;

    // Wait for warning to fire.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Drain and verify priority.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    let warning_entry = entries
        .iter()
        .find(|e| {
            matches!(
                e,
                closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _)
                    if text.contains("超时预警")
            )
        })
        .expect("should have a warning notification");

    match warning_entry {
        closeclaw_session::llm_session::QueueEntry::SystemNotification(_, priority) => {
            assert_eq!(
                *priority,
                NotificationPriority::Next,
                "yield warning must use Next priority (documented), got: {:?}",
                priority
            );
        }
        _ => unreachable!(),
    }

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}

// ══════════════════════════════════════════════════════════════════════════
// No children: warning notification still contains all 5 structural items
// ══════════════════════════════════════════════════════════════════════════

/// Even without children, the warning notification should contain
/// the 5 structural items (items 4/5 are omitted from per-child list
/// but the overall items 1-3 are present).
#[tokio::test]
#[serial]
async fn test_warning_no_children_structural_items() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-ncs").await;

    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 5, Some(1), None)
        .await;

    // Wait for warning.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let entries = {
        let mut cs_write = cs.write().await;
        cs_write.drain_queue()
    };

    let warning_text = entries
        .iter()
        .filter_map(|e| match e {
            closeclaw_session::llm_session::QueueEntry::SystemNotification(text, _) => {
                if text.contains("超时预警") {
                    Some(text.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .next()
        .expect("should have warning");

    // Item 1: timeout_warning_secs.
    assert!(
        warning_text.contains("设定预期执行时长: 1 秒"),
        "warning should have timeout_warning_secs, got: {}",
        warning_text
    );

    // Item 3: hard timeout & remaining.
    assert!(
        warning_text.contains("硬超时时间: 5 秒"),
        "warning should have hard timeout, got: {}",
        warning_text
    );
    assert!(
        warning_text.contains("剩余:"),
        "warning should have remaining, got: {}",
        warning_text
    );

    // No children indicator.
    assert!(
        warning_text.contains("无子 session"),
        "warning with no children should show '(无子 session)', got: {}",
        warning_text
    );

    // Cleanup.
    m.cancel_yield_timeout(&parent_id).await;
}
