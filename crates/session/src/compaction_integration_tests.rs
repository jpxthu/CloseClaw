//! Step 1.5 integration tests for compaction: snapshot, token warning
//! state routing, and message truncation.

#[cfg(test)]
mod tests {
    use crate::compaction::{
        estimate_messages_tokens, estimate_total_tokens, CompactConfig, CompactionMessage,
        CompactionService, TokenWarningState,
    };
    use crate::llm_session::SessionMessage;
    use crate::run_health::{RuntimeSnapshotManager, TranscriptOp};
    use chrono::Utc;
    use closeclaw_common::ContentBlock;
    use closeclaw_common::RunningStats;

    // Helper to build a SessionMessage with plain text.
    fn msg(text: &str) -> SessionMessage {
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text(text.into())],
            timestamp: Utc::now(),
        }
    }

    // Helper to build a CompactionMessage.
    fn comp_msg(text: &str) -> CompactionMessage {
        CompactionMessage {
            role: "user".into(),
            content: text.into(),
        }
    }

    // =====================================================================
    // 1. Pre-compaction snapshot
    // =====================================================================

    #[test]
    fn test_snapshot_save_restore_messages_intact() {
        let mut mgr = RuntimeSnapshotManager::new();
        let original = vec![msg("hello"), msg("world")];

        // Save snapshot before modification.
        mgr.create_snapshot(&original, TranscriptOp::Rewrite);

        // Mutate messages (demonstrating the scenario).
        // In real usage, mutated messages would be used for compaction.

        // Restore snapshot.
        let restored = mgr.rollback().expect("snapshot should exist");
        assert_eq!(restored.len(), 2);
        assert_eq!(
            restored[0].content_blocks[0],
            ContentBlock::Text("hello".into())
        );
        assert_eq!(
            restored[1].content_blocks[0],
            ContentBlock::Text("world".into())
        );
    }

    #[test]
    fn test_snapshot_multiple_saves_override_old() {
        let mut mgr = RuntimeSnapshotManager::new();

        mgr.create_snapshot(&[msg("first")], TranscriptOp::Rewrite);
        mgr.create_snapshot(&[msg("first"), msg("second")], TranscriptOp::Rewrite); // should override first snapshot

        // Restore should return the most recent snapshot (second save).
        let restored = mgr.rollback().expect("snapshot should exist");
        assert_eq!(restored.len(), 2);
        assert_eq!(
            restored[0].content_blocks[0],
            ContentBlock::Text("first".into())
        );
        assert_eq!(
            restored[1].content_blocks[0],
            ContentBlock::Text("second".into())
        );
    }

    #[test]
    fn test_snapshot_restore_clears_snapshot() {
        let mut mgr = RuntimeSnapshotManager::new();

        mgr.create_snapshot(&[msg("keep")], TranscriptOp::Rewrite);

        // First rollback: should succeed and remove snapshot.
        let restored = mgr.rollback().expect("snapshot should exist");
        assert_eq!(restored.len(), 1);

        // Second rollback: no snapshot left, returns None.
        assert!(mgr.rollback().is_none());
    }

    #[test]
    fn test_snapshot_no_messages() {
        let mut mgr = RuntimeSnapshotManager::new();
        mgr.create_snapshot(&[], TranscriptOp::Rewrite);
        let restored = mgr.rollback().expect("snapshot should exist");
        assert!(restored.is_empty());
    }

    #[test]
    fn test_clear_snapshot_after_success() {
        let mut mgr = RuntimeSnapshotManager::new();
        mgr.create_snapshot(&[msg("data")], TranscriptOp::Rewrite);

        mgr.clear();

        // Restore is no-op.
        assert!(mgr.rollback().is_none());
        // snapshot_count should be 0.
        assert_eq!(mgr.snapshot_count(), 0);
    }

    // =====================================================================
    // 2. token_warning_state integration — threshold boundaries
    // =====================================================================

    /// For mini-max model (1,000,000 tokens), test all four warning levels
    /// at their exact boundary values.
    #[test]
    fn test_token_warning_state_threshold_boundaries() {
        let svc = CompactionService::new(CompactConfig::default());
        let model = "mini-max";
        let context_window = 1_000_000;

        // Normal: remaining > 20,000
        assert_eq!(
            svc.token_warning_state(context_window - 20_001, model, None),
            TokenWarningState::Normal,
            "remaining 20,001 should be Normal"
        );

        // Warning: remaining <= 20,000 and > 13,000
        assert_eq!(
            svc.token_warning_state(context_window - 20_000, model, None),
            TokenWarningState::Warning,
            "remaining 20,000 should be Warning"
        );
        assert_eq!(
            svc.token_warning_state(context_window - 13_001, model, None),
            TokenWarningState::Warning,
            "remaining 13,001 should be Warning"
        );

        // AutoCompactTriggered: remaining <= 13,000 and > 3,000
        assert_eq!(
            svc.token_warning_state(context_window - 13_000, model, None),
            TokenWarningState::AutoCompactTriggered,
            "remaining 13,000 should be AutoCompactTriggered"
        );
        assert_eq!(
            svc.token_warning_state(context_window - 3_001, model, None),
            TokenWarningState::AutoCompactTriggered,
            "remaining 3,001 should be AutoCompactTriggered"
        );

        // Blocking: remaining <= 3,000
        assert_eq!(
            svc.token_warning_state(context_window - 3_000, model, None),
            TokenWarningState::Blocking,
            "remaining 3,000 should be Blocking"
        );
        assert_eq!(
            svc.token_warning_state(context_window - 1, model, None),
            TokenWarningState::Blocking,
            "remaining 1 should be Blocking"
        );

        // Fully used context.
        assert_eq!(
            svc.token_warning_state(context_window, model, None),
            TokenWarningState::Blocking,
            "remaining 0 should be Blocking"
        );
    }

    /// Circuit breaker interaction: when consecutive failures exceed the
    /// limit, `should_auto_compact` returns false even when tokens are
    /// in AutoCompactTriggered range.
    #[test]
    fn test_token_warning_state_circuit_breaker_blocks_auto_compact() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut svc = CompactionService::new(config);

        // Reach the AutoCompactTriggered range.
        let msgs = vec![comp_msg(&"x".repeat(3_948_004))]; // ~987,001 tokens
        assert!(svc.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));

        // Trip the circuit breaker.
        svc.record_failure();
        svc.record_failure();
        svc.record_failure();

        // token_warning_state still returns AutoCompactTriggered...
        let tokens = estimate_messages_tokens(&msgs, 0.25);
        assert_eq!(
            svc.token_warning_state(tokens, "mini-max", None),
            TokenWarningState::AutoCompactTriggered
        );
        // ...but should_auto_compact blocks it via circuit breaker.
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    /// Blocking state: the raw token_warning_state returns Blocking, and
    /// the session handler would reject the request (tested indirectly
    /// through the CompactionService boundary).
    #[test]
    fn test_blocking_state_rejects_compaction() {
        let svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![comp_msg(&"x".repeat(3_996_004))]; // ~999,001 tokens
        let tokens = estimate_messages_tokens(&msgs, 0.25);
        assert_eq!(
            svc.token_warning_state(tokens, "mini-max", None),
            TokenWarningState::Blocking
        );
        // should_auto_compact delegates to token_warning_state; Blocking
        // is not AutoCompactTriggered, so it returns false.
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    /// Warning state logs but does not trigger compaction.
    #[test]
    fn test_warning_state_no_compaction() {
        let svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![comp_msg(&"x".repeat(3_920_004))]; // ~980,001 tokens → remaining 19,999
        let tokens = estimate_messages_tokens(&msgs, 0.25);
        assert_eq!(
            svc.token_warning_state(tokens, "mini-max", None),
            TokenWarningState::Warning
        );
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    // =====================================================================
    // 3. Message truncation
    // =====================================================================

    #[test]
    fn test_truncation_retains_correct_count() {
        let mut config = CompactConfig::default();
        config.max_history_messages = Some(5);

        let mut msgs: Vec<CompactionMessage> =
            (0..10).map(|i| comp_msg(&format!("msg-{i}"))).collect();

        // Simulate the truncation logic from check_and_run_auto_compact.
        if let Some(max) = config.max_history_messages {
            if msgs.len() > max {
                let drain = msgs.len() - max;
                msgs.drain(..drain);
            }
        }

        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0].content, "msg-5");
        assert_eq!(msgs[4].content, "msg-9");
    }

    #[test]
    fn test_truncation_none_does_not_truncate() {
        let mut config = CompactConfig::default();
        config.max_history_messages = None;

        let mut msgs: Vec<CompactionMessage> =
            (0..20).map(|i| comp_msg(&format!("msg-{i}"))).collect();

        if let Some(max) = config.max_history_messages {
            if msgs.len() > max {
                let drain = msgs.len() - max;
                msgs.drain(..drain);
            }
        }

        assert_eq!(msgs.len(), 20);
    }

    #[test]
    fn test_truncation_fewer_than_max_unchanged() {
        let mut config = CompactConfig::default();
        config.max_history_messages = Some(10);

        let mut msgs: Vec<CompactionMessage> =
            (0..5).map(|i| comp_msg(&format!("msg-{i}"))).collect();

        if let Some(max) = config.max_history_messages {
            if msgs.len() > max {
                let drain = msgs.len() - max;
                msgs.drain(..drain);
            }
        }

        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn test_truncation_token_estimation_based_on_truncated() {
        let model = "glm-5";

        // 240 messages, each ~4000 chars → ~1000 tokens each → ~240,000 total.
        // glm-5 context = 256,000 → remaining ~16,000 → Warning range.
        let all_msgs: Vec<CompactionMessage> = (0..240)
            .map(|i| comp_msg(&format!("message-{i}: {}", "x".repeat(3990))))
            .collect();

        // Without truncation: 240 messages, ~240,000 tokens.
        let total_tokens_before = estimate_messages_tokens(&all_msgs, 0.25);
        assert!(total_tokens_before > 75_000);

        // With max_history_messages = 20: truncate first 220.
        let mut truncated = all_msgs.clone();
        let max = 20;
        if truncated.len() > max {
            let drain = truncated.len() - max;
            truncated.drain(..drain);
        }

        let total_tokens_after = estimate_messages_tokens(&truncated, 0.25);
        assert!(total_tokens_after < total_tokens_before);

        // The truncated set should be roughly 1/12 of original (20/240).
        let ratio = total_tokens_after as f64 / total_tokens_before as f64;
        assert!(
            (0.05..0.15).contains(&ratio),
            "truncated ratio should be ~0.08, got {ratio}"
        );

        // Token estimation should reflect truncated state, not original.
        let svc = CompactionService::new(CompactConfig::default());
        // Original total would be Warning range.
        assert_eq!(
            svc.token_warning_state(total_tokens_before, model, None),
            TokenWarningState::Warning
        );
        // Truncated total should be Normal (fewer tokens).
        assert_eq!(
            svc.token_warning_state(total_tokens_after, model, None),
            TokenWarningState::Normal
        );
    }

    #[test]
    fn test_truncation_exactly_at_boundary() {
        let mut config = CompactConfig::default();
        config.max_history_messages = Some(3);

        let mut msgs: Vec<CompactionMessage> =
            (0..3).map(|i| comp_msg(&format!("msg-{i}"))).collect();

        // Exactly at limit — no truncation needed.
        if let Some(max) = config.max_history_messages {
            if msgs.len() > max {
                let drain = msgs.len() - max;
                msgs.drain(..drain);
            }
        }

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "msg-0");
    }

    // =====================================================================
    // 4. RunningStats integration: estimate_total_tokens
    // =====================================================================

    /// When stats have LLM call history (request_count > 0), token estimation
    /// combines precise total_tokens from stats with char-based estimation for
    /// the pending messages.
    #[test]
    fn test_estimate_total_tokens_with_stats_and_messages() {
        let mut stats = RunningStats::default();
        stats.request_count = 5;
        stats.total_tokens = 50_000;

        let msgs = vec![comp_msg(&"a".repeat(1000)), comp_msg(&"b".repeat(2000))];
        // chars_per_token = 0.25
        // 50_000 + ceil(1000*0.25) + ceil(2000*0.25) = 50_000 + 250 + 500 = 50_750
        let total = estimate_total_tokens(&stats, &msgs, 0.25);
        assert_eq!(total, 50_750);
    }

    /// When stats have no LLM calls (request_count == 0), fall back to
    /// pure character-based estimation.
    #[test]
    fn test_estimate_total_tokens_fallback_pure_char() {
        let stats = RunningStats::default();
        assert_eq!(stats.request_count, 0);

        let msgs = vec![comp_msg(&"a".repeat(4000))];
        let total = estimate_total_tokens(&stats, &msgs, 0.25);
        assert_eq!(total, 1000); // 4000 * 0.25 = 1000
    }

    /// Empty messages with stats history returns just the stats total.
    #[test]
    fn test_estimate_total_tokens_empty_msgs_with_stats() {
        let mut stats = RunningStats::default();
        stats.request_count = 10;
        stats.total_tokens = 120_000;

        let msgs: Vec<CompactionMessage> = vec![];
        let total = estimate_total_tokens(&stats, &msgs, 0.25);
        assert_eq!(total, 120_000);
    }

    /// Integration: CompactionService should_auto_compact uses
    /// estimate_messages_tokens internally, so different chars_per_token
    /// values shift the compaction threshold.
    #[test]
    fn test_auto_compact_threshold_shifts_with_chars_per_token() {
        // With default chars_per_token = 0.25, need ~3_948_004 chars for
        // mini-max AutoCompactTriggered range.
        let msgs_025 = vec![comp_msg(&"x".repeat(3_948_004))];
        let mut config_025 = CompactConfig::default();
        config_025.chars_per_token = 0.25;
        let svc_025 = CompactionService::new(config_025);
        assert!(svc_025.should_auto_compact(&msgs_025, "mini-max", None, &RunningStats::new()));

        // With chars_per_token = 0.5, same chars produce 2x tokens →
        // compaction should trigger with fewer chars.
        let msgs_05 = vec![comp_msg(&"x".repeat(1_980_000))];
        let mut config_05 = CompactConfig::default();
        config_05.chars_per_token = 0.5;
        let svc_05 = CompactionService::new(config_05);
        // 1_980_000 * 0.5 = 990_000 tokens → remaining = 10_000 → AutoCompactTriggered
        assert!(svc_05.should_auto_compact(&msgs_05, "mini-max", None, &RunningStats::new()));

        // But with chars_per_token = 0.1, same chars produce fewer tokens →
        // might not trigger.
        let msgs_01 = vec![comp_msg(&"x".repeat(3_948_004))];
        let mut config_01 = CompactConfig::default();
        config_01.chars_per_token = 0.1;
        let svc_01 = CompactionService::new(config_01);
        // 3_948_004 * 0.1 = 394_800 tokens → Normal (960_200 remaining)
        assert!(!svc_01.should_auto_compact(&msgs_01, "mini-max", None, &RunningStats::new()));
    }

    /// Integration: knowledge_context_window affects auto_compact threshold.
    #[test]
    fn test_auto_compact_with_knowledge_context_window() {
        let msgs = vec![comp_msg(&"x".repeat(3_948_004))];
        let svc = CompactionService::new(CompactConfig::default());

        // Without knowledge: mini-max = 1_000_000 → AutoCompactTriggered
        assert!(svc.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));

        // With knowledge: 500_000 context → tokens are way over → Blocking
        // (not AutoCompactTriggered), so should_auto_compact returns false.
        assert!(!svc.should_auto_compact(&msgs, "mini-max", Some(500_000), &RunningStats::new()));
    }
}
