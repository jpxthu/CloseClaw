//! Integration tests for the session-side cache hit consistency check
//! (prefix-snapshot staging + eligibility judgment + unified advancement).
//!
//! Behavior dimensions (plan Step 1.3):
//! 1. First round / missing reference → new baseline, not checked
//! 2. Prefix preserved (tail-append) + deviation beyond tolerance → anomaly
//! 3. Prefix preserved + deviation within tolerance → no anomaly
//! 4. System prompt change / mid-history rewrite → exempt
//! 5. Model switch → exempt, snapshot advances, next round compares
//!    against the switched round
//! 6. Interval beyond the provider cache TTL → exempt (boundary participates)
//! 7. Advancement timing: exempt rounds advance; compaction-style rounds
//!    (no staging) never advance; invoke only stages; the check consumes
//!    the staging slot

use std::sync::Arc;

use async_trait::async_trait;

use crate::llm_session::ConversationSession;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{
    CacheAnomalyInfo, ContentBlock, InternalMessage, LLMError, LlmCaller, UnifiedResponse,
    UnifiedUsage,
};

use super::{make_usage_basic as make_usage, now_ms, snapshot, tmp_path, user_msg, TTL_MS};

// ---------------------------------------------------------------------------

/// Growing tail-append message list: ["turn-1", ..., "turn-n"].
fn turns(n: usize) -> Vec<InternalMessage> {
    (1..=n).map(|i| user_msg(&format!("turn-{i}"))).collect()
}

fn make_session() -> ConversationSession {
    ConversationSession::new("cache-prefix-check".into(), "test-model".into(), tmp_path())
}

/// Mirrors the production flow: stage (request build) → judge + advance
/// (usage fully arrived) → accumulate, with the same model identifier
/// passed to the check and the accumulation (gateway contract).
fn run_round(
    session: &mut ConversationSession,
    system: &str,
    msgs: Vec<InternalMessage>,
    usage: &UnifiedUsage,
    model: Option<&str>,
    now: u64,
) -> Option<CacheAnomalyInfo> {
    session.stage_cache_prefix_snapshot(Some(system), None, None, &msgs);
    let info = session.check_cache_anomaly_and_advance(usage, model, now, TTL_MS);
    session.accumulate_usage(usage, model);
    info
}

/// Fake caller returning a canned response (default trait `call_with_target`
/// reports no target — fake-path degradation must not panic).
struct EchoCaller;

#[async_trait]
impl LlmCaller for EchoCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("ok".into())],
            usage: make_usage(1_000, 50, Some(1_000)),
            finish_reason: Some("stop".into()),
            retry_attempts: 0,
        })
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<closeclaw_common::StreamEvent, LLMError>> + Send>,
        >,
        LLMError,
    > {
        Err(LLMError::ApiError("not implemented".into()))
    }
}

// ---------------------------------------------------------------------------
// Participation & numeric check
// ---------------------------------------------------------------------------

/// 首轮/参考值缺失 → 新基线不校验，但快照推进。
#[test]
fn test_first_round_is_new_baseline_exempt_but_advances() {
    let mut session = make_session();
    let info = run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
    );
    assert!(
        info.is_none(),
        "first round (no previous snapshot) must be a new baseline"
    );
    assert_eq!(
        session.cache_prefix_prev_snapshot(),
        Some(&snapshot("sys", &turns(1))),
        "first round advances the snapshot despite the exemption"
    );
}

/// 前缀一致（仅尾部追加）且偏差在容忍内 → 参与校验但不报异常。
#[test]
fn test_append_extension_within_tolerance_no_anomaly() {
    let mut session = make_session();
    let first = run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    assert!(first.is_none());
    let second = run_round(
        &mut session,
        "sys",
        turns(2),
        &make_usage(101_000, 100, Some(100_500)),
        Some("p/m"),
        now_ms(),
    );
    assert!(
        second.is_none(),
        "tail-append round with deviation 500 ≤ 1024 must not report an anomaly"
    );
}

/// 前缀一致（仅尾部追加）且偏差超限 → 报异常。
#[test]
fn test_append_extension_beyond_tolerance_reports_anomaly() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    let info = run_round(
        &mut session,
        "sys",
        turns(2),
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
    );
    let info = info.expect("whole-round miss on a comparable round must be reported");
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
    assert_eq!(info.tolerance_tokens, 1024);
}

/// 参考值缺失（合成为 0）→ 数值比较自身豁免（doc :81）。
#[test]
fn test_zero_reference_value_not_checked() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(0, 100, None),
        Some("p/m"),
        now_ms(),
    );
    let info = run_round(
        &mut session,
        "sys",
        turns(2),
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
    );
    assert!(
        info.is_none(),
        "a zero reference value must not enter the numeric check"
    );
}

// ---------------------------------------------------------------------------
// Prefix exemptions
// ---------------------------------------------------------------------------

/// System Prompt 变化 → 前缀不一致豁免；豁免轮推进快照，下一轮恢复校验。
#[test]
fn test_system_change_exempt_then_next_round_recheck() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys-v1",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    // System prompt changes → exempt even on a whole-round miss.
    let exempt = run_round(
        &mut session,
        "sys-v2",
        turns(2),
        &make_usage(101_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
    );
    assert!(
        exempt.is_none(),
        "system prompt change must exempt the round"
    );
    assert_eq!(
        session.cache_prefix_prev_snapshot(),
        Some(&snapshot("sys-v2", &turns(2))),
        "exempt round must still advance the snapshot"
    );
    // Next round under the new system → checked against the exempt round.
    let info = run_round(
        &mut session,
        "sys-v2",
        turns(3),
        &make_usage(101_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
    );
    let info = info.expect("round after the exempt round must be checked against it");
    assert_eq!(
        info.previous_prompt_tokens, 101_000,
        "reference value must be the exempt round's prompt tokens"
    );
}

/// 消息中部改写（同长度不同内容）→ 前缀不一致豁免。
#[test]
fn test_mid_history_rewrite_exempt() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(2),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    let mut rewritten = turns(2);
    rewritten[0] = user_msg("turn-1 REWRITTEN");
    session.stage_cache_prefix_snapshot(Some("sys"), None, None, &rewritten);
    let info = session.check_cache_anomaly_and_advance(
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        now_ms(),
        TTL_MS,
    );
    assert!(info.is_none(), "rewritten history must exempt the round");
    session.accumulate_usage(&make_usage(100_000, 100, Some(0)), Some("p/m"));
}

// ---------------------------------------------------------------------------
// Model-switch exemption & TTL exemption
// ---------------------------------------------------------------------------

/// 模型切换 → 豁免且快照推进；下一轮与切换后这轮比较。
#[test]
fn test_model_switch_exempt_and_next_round_compares_to_switched_round() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/model-a"),
        now_ms(),
    );
    // Switch to model b → exempt despite a whole-round miss.
    let exempt = run_round(
        &mut session,
        "sys",
        turns(2),
        &make_usage(101_000, 100, Some(0)),
        Some("p/model-b"),
        now_ms(),
    );
    assert!(exempt.is_none(), "model switch must exempt the round");
    assert_eq!(
        session.cache_prefix_prev_snapshot(),
        Some(&snapshot("sys", &turns(2))),
        "model-switch exempt round must still advance the snapshot"
    );
    // Round 3 on model b → compared against round 2 (the switched round).
    let info = run_round(
        &mut session,
        "sys",
        turns(3),
        &make_usage(101_000, 100, Some(0)),
        Some("p/model-b"),
        now_ms(),
    );
    let info = info.expect("round after a model switch must be checked against the switched round");
    assert_eq!(info.previous_prompt_tokens, 101_000);
}

/// 间隔超 TTL → 豁免（即使整轮未命中也不报）。
#[test]
fn test_interval_beyond_ttl_exempt() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    // Rewind the previous-call timestamp to a fixed past point.
    session.stats_mut().last_call_at = Some(1_000);
    session.stats_mut().last_call_model = Some("p/m".to_string());
    session.stage_cache_prefix_snapshot(Some("sys"), None, None, &turns(2));
    let info = session.check_cache_anomaly_and_advance(
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        1_000 + TTL_MS + 1,
        TTL_MS,
    );
    assert!(
        info.is_none(),
        "interval beyond TTL must exempt the round even on a whole-round miss"
    );
}

/// 间隔恰好等于 TTL（未超过）→ 参与：整轮未命中被判定为异常。
#[test]
fn test_interval_exactly_ttl_participates() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    session.stats_mut().last_call_at = Some(1_000);
    session.stats_mut().last_call_model = Some("p/m".to_string());
    session.stage_cache_prefix_snapshot(Some("sys"), None, None, &turns(2));
    let info = session.check_cache_anomaly_and_advance(
        &make_usage(100_000, 100, Some(0)),
        Some("p/m"),
        1_000 + TTL_MS,
        TTL_MS,
    );
    assert!(
        info.is_some(),
        "interval == TTL has not expired: the round participates and the miss is reported"
    );
}

// ---------------------------------------------------------------------------
// Advancement-timing audit
// ---------------------------------------------------------------------------

/// 推进仅发生在统一推进点：invoke 仅暂存不推进；check 消费暂存槽并推进。
#[tokio::test]
async fn test_invoke_stages_without_advancing_check_advances() {
    let mut session = make_session().with_system_prompt("static system");
    session.set_llm_caller(Arc::new(EchoCaller));

    // Request build (invoke path) stages the snapshot, never advances.
    let response = session.invoke_llm("hello").await.unwrap();
    assert_eq!(response.usage.prompt_tokens, 1_000);
    assert!(
        session.cache_prefix_prev_snapshot().is_none(),
        "request build must only stage, never advance"
    );
    assert!(
        session.staged_cache_prefix_snapshot().is_some(),
        "invoke must stage this round's snapshot"
    );

    // Unified advancement point: judges (first round → baseline) + advances.
    let usage = make_usage(1_000, 50, Some(1_000));
    let info = session.check_cache_anomaly_and_advance(&usage, Some("p/m"), now_ms(), TTL_MS);
    assert!(info.is_none(), "first round is a new baseline");
    assert!(
        session.cache_prefix_prev_snapshot().is_some(),
        "the unified point must advance the snapshot"
    );
    assert!(
        session.staged_cache_prefix_snapshot().is_none(),
        "the check consumes the staging slot"
    );
}

/// 压缩调用不经会话调用路径（无暂存）→ 不推进快照。
#[test]
fn test_compaction_style_round_without_staging_does_not_advance() {
    let mut session = make_session();
    run_round(
        &mut session,
        "sys",
        turns(1),
        &make_usage(100_000, 100, Some(100_000)),
        Some("p/m"),
        now_ms(),
    );
    let before = session.cache_prefix_prev_snapshot().cloned();
    // Compaction-style round: the chat_fn path never stages.
    let info = session.check_cache_anomaly_and_advance(
        &make_usage(5_000, 200, None),
        Some("p/m"),
        now_ms(),
        TTL_MS,
    );
    assert!(info.is_none());
    assert_eq!(
        session.cache_prefix_prev_snapshot().cloned(),
        before,
        "a round without staging must not advance the snapshot"
    );
}

/// 新会话无任何快照状态（运行时态不跨会话泄漏）。
#[test]
fn test_fresh_session_has_no_snapshot_state() {
    let mut session = make_session();
    assert!(session.cache_prefix_prev_snapshot().is_none());
    assert!(session
        .check_cache_anomaly_and_advance(&make_usage(1, 1, None), None, now_ms(), TTL_MS)
        .is_none());
}
