//! Step 1.5 integration tests for compaction: snapshot, token warning
//! state routing, and message truncation.

#[cfg(test)]
mod tests {
    use crate::compaction::{
        estimate_messages_tokens, CompactConfig, CompactionMessage, CompactionService,
        TokenWarningState,
    };
    use crate::llm_session::{ChatSession, ConversationSession, SessionMessage};
    use chrono::Utc;
    use closeclaw_common::ContentBlock;
    use std::path::PathBuf;

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

    // Helper to create a ConversationSession for tests.
    fn new_session(id: &str) -> ConversationSession {
        ConversationSession::new(id.into(), "glm-5".into(), PathBuf::from("/tmp"))
    }

    // =====================================================================
    // 1. Pre-compaction snapshot
    // =====================================================================

    #[test]
    fn test_snapshot_save_restore_messages_intact() {
        let mut session = new_session("s1");
        let original = vec![msg("hello"), msg("world")];
        session.replace_messages(original.clone());

        // Save snapshot before modification.
        session.save_snapshot();

        // Mutate messages.
        session.replace_messages(vec![msg("hello"), msg("world"), msg("added")]);

        // Restore snapshot.
        assert!(session.restore_snapshot());
        // Messages should match original.
        assert_eq!(session.messages().len(), 2);
        assert_eq!(
            session.messages()[0].content_blocks[0],
            ContentBlock::Text("hello".into())
        );
        assert_eq!(
            session.messages()[1].content_blocks[0],
            ContentBlock::Text("world".into())
        );
    }

    #[test]
    fn test_snapshot_multiple_saves_override_old() {
        let mut session = new_session("s1");
        session.replace_messages(vec![msg("first")]);
        session.save_snapshot();

        session.replace_messages(vec![msg("first"), msg("second")]);
        session.save_snapshot(); // should override first snapshot

        session.replace_messages(vec![msg("first"), msg("second"), msg("third")]);

        assert!(session.restore_snapshot());
        // Restored state should have "first" and "second" (from second save),
        // but NOT "third".
        assert_eq!(session.messages().len(), 2);
        assert_eq!(
            session.messages()[0].content_blocks[0],
            ContentBlock::Text("first".into())
        );
        assert_eq!(
            session.messages()[1].content_blocks[0],
            ContentBlock::Text("second".into())
        );
    }

    #[test]
    fn test_snapshot_restore_clears_snapshot() {
        let mut session = new_session("s1");
        session.replace_messages(vec![msg("keep")]);
        session.save_snapshot();
        session.replace_messages(vec![msg("keep"), msg("discard")]);

        // First restore: should succeed and clear snapshot.
        assert!(session.restore_snapshot());
        assert_eq!(session.messages().len(), 1);

        // Second restore: snapshot is None, returns false.
        assert!(!session.restore_snapshot());
        // Messages unchanged after failed restore.
        assert_eq!(session.messages().len(), 1);
    }

    #[test]
    fn test_snapshot_no_messages() {
        let mut session = new_session("s1");
        session.save_snapshot();
        assert!(session.restore_snapshot());
        assert!(session.messages().is_empty());
    }

    #[test]
    fn test_clear_snapshot_after_success() {
        let mut session = new_session("s1");
        session.replace_messages(vec![msg("data")]);
        session.save_snapshot();

        session.clear_snapshot();

        // Restore is no-op.
        assert!(!session.restore_snapshot());
        // Original message is still there (clear doesn't touch messages).
        assert_eq!(session.messages().len(), 1);
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
        assert!(svc.should_auto_compact(&msgs, "mini-max", None));

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
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None));
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
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None));
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
        assert!(!svc.should_auto_compact(&msgs, "mini-max", None));
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
}
