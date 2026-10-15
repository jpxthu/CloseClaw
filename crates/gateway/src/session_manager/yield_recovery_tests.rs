//! Tests for yield recovery: auto-recovery when all children complete,
//! announce injection, timeout cancellation, and message queue drain.
//!
//! Covers Step 1.6 (Waiting 消息排队与自动恢复) and integration
//! scenarios across Steps 1.5–1.7.

use super::spawn::SpawnMode;
use super::test_helpers::{
    append_assistant_to_child, setup_parent_with_conv, spawn_n_run_children,
    test_spawn_target_config,
};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_common::NotificationPriority;
use closeclaw_llm::types::{InternalRequest, StreamEvent, UnifiedResponse, UnifiedUsage};
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::persistence::PendingMessage;
use serial_test::serial;

// ── Helper: complete a child and remove it from the SpawnTree ──────────────

async fn complete_and_remove_child(
    mgr: &super::SessionManager,
    child_id: &str,
    parent_id: &str,
    blocks: Vec<closeclaw_llm::types::ContentBlock>,
) {
    append_assistant_to_child(mgr, child_id, blocks).await;
    mgr.try_push_announce(child_id, NotificationPriority::Next)
        .await;
    mgr.children.write().await.remove_child(parent_id, child_id);
}

// ── 1. Single child: recovery after completion + removal ───────────────────

/// When the last run-mode child completes and is removed from the tree,
/// triggering recovery exits Waiting.
#[tokio::test]
#[serial]
async fn test_yield_recovery_single_child() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-recovery").await;

    let child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-r1", None),
            &parent_id,
            1,
            "do task",
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
            None, // prompt_template_prefix
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    // Complete child, push announce, and remove from tree.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".into())],
    )
    .await;

    // Trigger recovery — now no run-mode children remain.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should exit Waiting after all children removed"
    );
}

// ── 2. Two children: recovery only after both are removed ──────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_two_children() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-multi").await;

    let child_ids = spawn_n_run_children(&mgr, &parent_id, 2).await;

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove first child.
    complete_and_remove_child(
        &mgr,
        &child_ids[0],
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("first".into())],
    )
    .await;

    // Still waiting — one child remaining.
    mgr.trigger_yield_recovery(&parent_id).await;
    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "parent should still be Waiting with one child remaining"
    );

    // Complete and remove second child.
    complete_and_remove_child(
        &mgr,
        &child_ids[1],
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("second".into())],
    )
    .await;

    // Now recovered.
    mgr.trigger_yield_recovery(&parent_id).await;
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should exit Waiting after all children removed"
    );
}

// ── 3. Recovery injects announce messages ──────────────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_injects_announce() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-inject").await;

    let child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-inj", None),
            &parent_id,
            1,
            "compute",
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
            None, // prompt_template_prefix
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove child.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text(
            "result: 42".into(),
        )],
    )
    .await;

    // Trigger recovery.
    mgr.trigger_yield_recovery(&parent_id).await;

    // Check that the announce was injected as a system message.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_announce = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("result: 42"))
            })
    });
    assert!(
        has_announce,
        "announce message should be injected into parent's history"
    );
}

// ── 4. No children at yield → immediate recovery ───────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_no_children_immediate_recovery() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-no-children").await;

    // Enter Waiting — no children spawned.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    // Trigger recovery — no run-mode children, should recover immediately.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent with no children should exit Waiting immediately"
    );
}

// ── 5. is_session_yielding tracks state correctly ──────────────────────────

#[tokio::test]
#[serial]
async fn test_is_session_yielding_tracks_state() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-state").await;

    assert!(!mgr.is_session_yielding(&parent_id).await);

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.exit_waiting();
    }
    assert!(!mgr.is_session_yielding(&parent_id).await);
}

// ── 6. Announce queue is drained during recovery ──────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_drains_announce_queue() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-drain").await;

    let child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-drain", None),
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
            None, // prompt_template_prefix
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove child.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("output".into())],
    )
    .await;

    // Trigger recovery.
    mgr.trigger_yield_recovery(&parent_id).await;

    // After recovery, the announce queue should be drained.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert!(
        remaining.is_empty(),
        "announce queue should be empty after recovery"
    );
}

// ── 7. Session-mode child doesn't block recovery ──────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_session_mode_no_block() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-sess").await;

    let child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-sess", None),
            &parent_id,
            1,
            "long task",
            true,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None, // prompt_template_prefix
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete session-mode child and remove from tree.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".into())],
    )
    .await;
    mgr.try_push_announce(&child_id, NotificationPriority::Next)
        .await;
    mgr.children
        .write()
        .await
        .remove_child(&parent_id, &child_id);

    // Trigger recovery — session-mode child doesn't block.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should recover after session-mode child is removed"
    );
}

// ── 8. Recovery does NOT happen while children remain ──────────────────────

/// While a run-mode child is still in the SpawnTree, recovery should
/// not trigger.
#[tokio::test]
#[serial]
async fn test_yield_no_recovery_while_child_registered() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-stay").await;

    let _child_id = mgr
        .create_child_session(
            &test_spawn_target_config("worker-stay", None),
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
            None, // prompt_template_prefix
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Trigger recovery — child still in tree.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "parent should remain in Waiting while child is registered"
    );
}

// ── 9. Recovery drain: unified cache-check advancement (Step 1.7 ①) ────────

/// Fake caller for the recovery-drain cache tests: every call answers
/// prompt 100 000 with a full cache hit (cache_read 100 000) except calls
/// numbered `miss_from_call` and later, which are whole-round misses
/// (cache_read 0). Reports no call target (default trait impl) —
/// consumers degrade to session-config semantics.
struct RecoveryCacheCaller {
    miss_from_call: usize,
    calls: std::sync::atomic::AtomicUsize,
}

impl RecoveryCacheCaller {
    /// Every round is a stable full hit.
    fn stable() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            miss_from_call: usize::MAX,
            calls: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// First round is a full hit, every later round a whole-round miss.
    fn miss_after_first() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            miss_from_call: 2,
            calls: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl closeclaw_common::LlmCaller for RecoveryCacheCaller {
    async fn call(
        &self,
        _request: InternalRequest,
    ) -> Result<UnifiedResponse, closeclaw_llm::LLMError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let cache_read = if n >= self.miss_from_call {
            Some(0)
        } else {
            Some(100_000)
        };
        Ok(UnifiedResponse {
            content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(format!(
                "resp-{n}"
            ))],
            usage: UnifiedUsage {
                prompt_tokens: 100_000,
                completion_tokens: 50,
                total_tokens: Some(100_050),
                reasoning_tokens: None,
                cache_read_tokens: cache_read,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
            retry_attempts: 0,
        })
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<StreamEvent, closeclaw_llm::LLMError>> + Send>,
        >,
        closeclaw_llm::LLMError,
    > {
        Err(closeclaw_llm::LLMError::ApiError("not implemented".into()))
    }
}

/// Queue one pending user message and run the recovery drain path.
async fn drain_round(mgr: &super::SessionManager, session_id: &str, content: &str) {
    mgr.push_pending_message(
        session_id,
        PendingMessage::new(format!("pend-{content}"), content.to_string()),
    )
    .await
    .expect("push pending message");
    mgr.drain_pending_for_session(session_id).await;
}

/// One main-path-shaped round: pre-append the user message to history
/// (dispatch semantics), invoke, then run the unified advancement
/// (check + accumulate) exactly as `clear_busy_and_send` does. Returns
/// the check's anomaly result.
async fn main_path_round(
    mgr: &super::SessionManager,
    session_id: &str,
    content: &str,
) -> Option<closeclaw_common::CacheAnomalyInfo> {
    let cs = mgr
        .get_conversation_session(session_id)
        .await
        .expect("conversation session");
    let (response, _target) = {
        let mut cs_write = cs.write().await;
        cs_write.append_user_message(content);
        cs_write
            .invoke_llm_with_target(content)
            .await
            .expect("main-path round invoke")
    };
    let mut cs_write = cs.write().await;
    cs_write.append_response(response.clone());
    let info = cs_write.check_cache_anomaly_and_advance(
        &response.usage,
        Some("test-model"),
        closeclaw_common::current_epoch_millis(),
        300_000,
    );
    cs_write.accumulate_usage(&response.usage, Some("test-model"));
    info
}

/// 恢复轮接入统一推进（三段链，模拟恢复后的真实轮次序列；round-2 实际
/// 豁免分支经临时探针实测裁决——`PrefixMismatch`，非静态推断）：
/// 1. 恢复轮（drain 路径）：pending 用户消息只进实际发送列表 [u1]、
///    不写历史，历史仅落响应；无上一轮快照 → NoPreviousSnapshot 新基线；
///    用量累积、参考值推进（doc :65/:73）；
/// 2. 恢复后首轮（主路径形态）：历史此时为 [resp-1]，append u2 后实际
///    发送列表 [resp-1, u2]——相对恢复轮快照 [u1] 首位即不同（恢复轮
///    用户消息不在历史），按定义非尾部追加 → PrefixMismatch 豁免；
///    参考值已推进，不产生陈旧参考值误报；
/// 3. 再下一轮（真尾部追加）：实际发送列表 [resp-1, u2, resp-2, u3]
///    以第 2 轮快照为前缀 → 参与校验——整轮未命中时以推进后的参考值
///    报异常，证明恢复轮的快照与参考值推进真实生效（若前缀判定整体
///    失效为恒匹配，round-2 参与校验即报假异常；恒不匹配则 round-3
///    豁免、expect 失败——两个回归方向均不绿）。
#[tokio::test]
#[serial]
async fn test_drain_pending_recovery_round_advances_reference_and_snapshot() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-recover-cache").await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.write()
            .await
            .set_llm_caller(RecoveryCacheCaller::miss_after_first());
    }

    // ── Round 1: recovery drain round ──
    drain_round(&mgr, &parent_id, "turn-1").await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let cs = cs.read().await;
        assert_eq!(
            cs.stats().request_count,
            1,
            "recovery round must accumulate usage (doc :65)"
        );
        assert_eq!(
            cs.stats().last_prompt_tokens,
            Some(100_000),
            "recovery round must set the next round's reference value"
        );
        assert_eq!(
            cs.stats().last_call_model.as_deref(),
            Some("test-model"),
            "no call target reported → degrade to session-config model"
        );
        // History lock: the drain round's user message is sent-list-only
        // (never enters the transcript) — this is what makes round-2's
        // sent list diverge from the recovery snapshot at position 0.
        let messages = cs.messages().to_vec();
        assert_eq!(
            messages.len(),
            1,
            "drain round writes only the response to history, not the \
             pending user message"
        );
        assert_eq!(messages[0].role, "assistant");
    }

    // ── Round 2: first post-recovery round, main-path shape ──
    // Sent list [resp-1, u2] vs recovery-round snapshot [u1]: first
    // position differs → not a tail-append → PrefixMismatch exemption.
    let info2 = main_path_round(&mgr, &parent_id, "turn-2").await;
    assert!(
        info2.is_none(),
        "round-2 sent list [resp-1, u2] is not a tail-append of the \
         recovery-round snapshot [u1]: exempt, no stale-reference false \
         positive"
    );

    // ── Round 3: tail-append extension of round 2 → participates ──
    let info3 = main_path_round(&mgr, &parent_id, "turn-3").await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let cs = cs.read().await;
        assert_eq!(
            cs.stats().request_count,
            3,
            "recovery round + both main-path rounds must accumulate"
        );
    }
    let info = info3.expect("tail-append round must be checked against the advanced reference");
    assert_eq!(
        info.previous_prompt_tokens, 100_000,
        "reference value must descend from the advanced chain, not a stale pre-recovery value"
    );
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
}

/// 恢复轮之后的稳定命中轮不误报：同三段链，全轮稳定命中 → 参与校验轮
/// 偏差为 0，不报异常。
#[tokio::test]
#[serial]
async fn test_drain_pending_round_after_recovery_no_false_positive() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-recover-cache-ok").await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.write()
            .await
            .set_llm_caller(RecoveryCacheCaller::stable());
    }

    drain_round(&mgr, &parent_id, "turn-1").await;
    let info2 = main_path_round(&mgr, &parent_id, "turn-2").await;
    assert!(
        info2.is_none(),
        "post-recovery round 2 is exempt (prefix mismatch vs the \
         recovery-round snapshot)"
    );
    let info3 = main_path_round(&mgr, &parent_id, "turn-3").await;
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let cs = cs.read().await;
        assert_eq!(cs.stats().request_count, 3);
        assert!(
            info3.is_none() && cs.last_cache_anomaly().is_none(),
            "stable full hit within tolerance after a recovery round must not be flagged"
        );
    }
}
