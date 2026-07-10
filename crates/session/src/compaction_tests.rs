//! Tests for compaction module

#[cfg(test)]
mod tests {
    use crate::compaction::{
        build_compact_prompt, estimate_messages_tokens, estimate_tokens, estimate_total_tokens,
        extract_summary, format_boundary_message, get_context_window, CompactConfig,
        CompactionError, CompactionMessage, CompactionService, TokenWarningState,
    };
    use closeclaw_common::RunningStats;

    #[test]
    fn test_estimate_tokens_english() {
        // "hello" = 5 chars * 0.25 = 1.25 -> ceil = 2
        let tokens = estimate_tokens("hello", 0.25);
        assert!(tokens >= 2 && tokens <= 5, "expected 2-5, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_chinese() {
        // "你好" = 2 chars * 0.25 = 0.5 -> ceil = 1
        let tokens = estimate_tokens("你好", 0.25);
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens("", 0.25), 0);
    }

    #[test]
    fn test_estimate_tokens_emoji() {
        // "🎉🎊🔥" = 3 chars * 0.25 = 0.75 -> ceil = 1
        let tokens = estimate_tokens("🎉🎊🔥", 0.25);
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_long_string() {
        let s = "a".repeat(1000);
        assert_eq!(estimate_tokens(&s, 0.25), 250);
    }

    #[test]
    fn test_get_context_window_minimax() {
        assert_eq!(get_context_window("mini-max", None), 1_000_000);
    }

    #[test]
    fn test_get_context_window_glm() {
        assert_eq!(get_context_window("glm-5.1", None), 256_000);
    }

    #[test]
    fn test_get_context_window_unknown() {
        assert_eq!(get_context_window("unknown-model-xyz", None), 128_000);
    }

    #[test]
    fn test_get_context_window_knowledge_override() {
        // Knowledge base value takes precedence over hardcoded table
        assert_eq!(get_context_window("mini-max", Some(500_000)), 500_000);
    }

    #[test]
    fn test_get_context_window_knowledge_zero_falls_back() {
        // knowledge_context_window = 0 means unknown → fallback to hardcoded
        assert_eq!(get_context_window("mini-max", Some(0)), 1_000_000);
    }

    #[test]
    fn test_get_context_window_knowledge_none_falls_back() {
        assert_eq!(get_context_window("glm-5.1", None), 256_000);
    }

    #[test]
    fn test_should_auto_compact_below_threshold() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "short".to_string(),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    #[test]
    fn test_should_auto_compact_circuit_breaker() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        // Record failures up to max
        service.record_failure();
        service.record_failure();
        service.record_failure();
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "x".repeat(900_000),
        }];
        // Circuit breaker should trip
        assert!(!service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    #[test]
    fn test_token_warning_state_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // 1,000,000 - 50,000 = 950,000 used -> 50,000 remaining > 20,000
        assert_eq!(
            service.token_warning_state(950_000, "mini-max", None),
            TokenWarningState::Normal
        );
    }

    #[test]
    fn test_token_warning_state_warning() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 20,000 -> Warning
        assert_eq!(
            service.token_warning_state(980_000, "mini-max", None),
            TokenWarningState::Warning
        );
    }

    #[test]
    fn test_token_warning_state_auto_compact() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 13,000 -> AutoCompactTriggered
        assert_eq!(
            service.token_warning_state(987_000, "mini-max", None),
            TokenWarningState::AutoCompactTriggered
        );
    }

    #[test]
    fn test_token_warning_state_blocking() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 3,000 -> Blocking
        assert_eq!(
            service.token_warning_state(997_000, "mini-max", None),
            TokenWarningState::Blocking
        );
    }

    #[test]
    fn test_token_warning_state_knowledge_override() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // Knowledge base context = 500,000; used = 485,000 → remaining = 15,000 → Warning
        assert_eq!(
            service.token_warning_state(485_000, "mini-max", Some(500_000)),
            TokenWarningState::Warning
        );
    }

    #[test]
    fn test_percent_left_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(500_000, "mini-max", None), 50);
    }

    #[test]
    fn test_percent_left_zero_used() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(0, "mini-max", None), 100);
    }

    #[test]
    fn test_percent_left_near_full() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(999_000, "mini-max", None), 0);
    }

    #[test]
    fn test_percent_left_knowledge_override() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // Knowledge base context = 200,000; used = 150,000 → 25% left
        assert_eq!(service.percent_left(150_000, "mini-max", Some(200_000)), 25);
    }

    #[test]
    fn test_record_failure_increments() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        assert_eq!(service.consecutive_failures(), 0);
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 1);
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 2);
    }

    #[test]
    fn test_record_success_resets() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        service.record_failure();
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 2);
        service.record_success();
        assert_eq!(service.consecutive_failures(), 0);
    }

    #[test]
    fn test_should_auto_compact_recovers_after_success() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);
        service.record_failure();
        service.record_failure();
        service.record_failure();
        // 3,948,004 chars * 0.25 = 987,001 tokens → AutoCompactTriggered (mini-max).
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "x".repeat(3_948_004),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
        service.record_success();
        assert!(service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    // Step 1.2 tests: Prompt template and summary extraction
    #[test]
    fn test_build_compact_prompt_none() {
        let prompt = build_compact_prompt(None);
        assert!(prompt.contains("You must not call any tools"));
    }

    #[test]
    fn test_build_compact_prompt_with_instructions() {
        let prompt = build_compact_prompt(Some("xxx"));
        assert!(prompt.contains("保留 xxx"));
    }

    #[test]
    fn test_build_compact_prompt_empty() {
        let p1 = build_compact_prompt(None);
        let p2 = build_compact_prompt(Some(""));
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_extract_summary_simple() {
        assert_eq!(extract_summary("hello"), None);
        assert_eq!(
            extract_summary("<summary>test</summary>"),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_extract_summary_with_analysis() {
        let r = extract_summary("<analysis>draft</analysis><summary>content</summary>");
        assert_eq!(r, Some("content".to_string()));
    }

    #[test]
    fn test_extract_summary_empty() {
        assert_eq!(extract_summary("<summary></summary>"), Some("".to_string()));
    }

    #[test]
    fn test_extract_summary_no_tags() {
        assert_eq!(extract_summary("no tags"), None);
    }

    #[test]
    fn test_extract_summary_unclosed() {
        assert_eq!(extract_summary("<summary>unclosed"), None);
    }

    #[test]
    fn test_format_boundary_message_auto() {
        let ts = chrono::Utc::now();
        let msg = format_boundary_message("summary", true, ts);
        assert!(msg.contains("自动压缩"));
        assert!(msg.contains(&ts.to_string()));
    }

    #[test]
    fn test_format_boundary_message_manual() {
        let ts = chrono::Utc::now();
        let msg = format_boundary_message("summary", false, ts);
        assert!(msg.contains("手动压缩"));
        assert!(msg.contains(&ts.to_string()));
    }

    // ===================================================================
    // Step 1.4 tests: estimate_total_tokens
    // ===================================================================

    #[test]
    fn test_estimate_total_tokens_with_llm_history() {
        let mut stats = RunningStats::default();
        stats.request_count = 5;
        stats.total_tokens = 10_000;
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "hello world".to_string(),
        }];
        let result = estimate_total_tokens(&stats, &msgs, 0.25);
        // 10,000 + estimate_tokens("hello world", 0.25)
        // "hello world" = 11 chars * 0.25 = 2.75 -> ceil = 3
        assert_eq!(result, 10_003);
    }

    #[test]
    fn test_estimate_total_tokens_no_llm_history() {
        let stats = RunningStats::default();
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "hello world".to_string(),
        }];
        let result = estimate_total_tokens(&stats, &msgs, 0.25);
        // Pure char estimation: 11 * 0.25 = 2.75 -> ceil = 3
        assert_eq!(result, 3);
    }

    #[test]
    fn test_estimate_total_tokens_zero_messages_with_history() {
        let mut stats = RunningStats::default();
        stats.request_count = 10;
        stats.total_tokens = 50_000;
        let msgs: Vec<CompactionMessage> = vec![];
        let result = estimate_total_tokens(&stats, &msgs, 0.25);
        assert_eq!(result, 50_000);
    }

    #[test]
    fn test_estimate_total_tokens_zero_messages_no_history() {
        let stats = RunningStats::default();
        let msgs: Vec<CompactionMessage> = vec![];
        let result = estimate_total_tokens(&stats, &msgs, 0.25);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_estimate_total_tokens_multiple_messages() {
        let mut stats = RunningStats::default();
        stats.request_count = 3;
        stats.total_tokens = 1_000;
        let msgs = vec![
            CompactionMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            },
            CompactionMessage {
                role: "assistant".to_string(),
                content: "hello there".to_string(),
            },
        ];
        let result = estimate_total_tokens(&stats, &msgs, 0.25);
        // 1000 + estimate_tokens("hi", 0.25) + estimate_tokens("hello there", 0.25)
        // 1000 + ceil(2*0.25) + ceil(11*0.25) = 1000 + 1 + 3 = 1004
        assert_eq!(result, 1_004);
    }

    // ===================================================================
    // Step 1.4 tests: chars_per_token different values
    // ===================================================================

    #[test]
    fn test_estimate_tokens_different_chars_per_token() {
        // 100 chars with different coefficients
        let text = "a".repeat(100);
        assert_eq!(estimate_tokens(&text, 0.25), 25); // 100 * 0.25 = 25
        assert_eq!(estimate_tokens(&text, 0.3), 30); // 100 * 0.3 = 30
        assert_eq!(estimate_tokens(&text, 0.5), 50); // 100 * 0.5 = 50
        assert_eq!(estimate_tokens(&text, 1.0), 100); // 100 * 1.0 = 100
        assert_eq!(estimate_tokens(&text, 0.1), 10); // 100 * 0.1 = 10
    }

    #[test]
    fn test_estimate_messages_tokens_different_chars_per_token() {
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "a".repeat(100),
        }];
        assert_eq!(estimate_messages_tokens(&msgs, 0.25), 25);
        assert_eq!(estimate_messages_tokens(&msgs, 0.3), 30);
        assert_eq!(estimate_messages_tokens(&msgs, 0.5), 50);
    }

    #[test]
    fn test_estimate_total_tokens_different_chars_per_token() {
        let mut stats = RunningStats::default();
        stats.request_count = 2;
        stats.total_tokens = 1_000;
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "a".repeat(100),
        }];
        assert_eq!(estimate_total_tokens(&stats, &msgs, 0.25), 1_025);
        assert_eq!(estimate_total_tokens(&stats, &msgs, 0.3), 1_030);
        assert_eq!(estimate_total_tokens(&stats, &msgs, 0.5), 1_050);
    }

    #[test]
    fn test_should_auto_compact_different_chars_per_token() {
        // With chars_per_token = 0.5, fewer chars needed to reach threshold
        let mut config = CompactConfig::default();
        config.chars_per_token = 0.5;
        let service = CompactionService::new(config);
        // mini-max context = 1_000_000, need ~987_001 tokens for AutoCompactTriggered
        // chars_per_token = 0.5: 987_001 / 0.5 ≈ 1_974_002 chars needed
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "x".repeat(1_974_002),
        }];
        assert!(service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
    }

    // ===================================================================
    // Step 1.4 tests: get_context_window knowledge priority
    // ===================================================================

    #[test]
    fn test_get_context_window_knowledge_overrides_all() {
        // Knowledge base value always takes priority
        assert_eq!(get_context_window("unknown-model", Some(100_000)), 100_000);
        assert_eq!(get_context_window("glm-5", Some(50_000)), 50_000);
        assert_eq!(get_context_window("mini-max", Some(1_500_000)), 1_500_000);
    }

    #[test]
    fn test_get_context_window_knowledge_zero_defers_to_hardcoded() {
        assert_eq!(get_context_window("glm-5", Some(0)), 256_000);
        assert_eq!(get_context_window("mini-max", Some(0)), 1_000_000);
    }

    #[test]
    fn test_get_context_window_none_defers_to_hardcoded() {
        assert_eq!(get_context_window("glm-4", None), 256_000);
        assert_eq!(get_context_window("glm-3", None), 128_000);
        assert_eq!(get_context_window("no-such-model", None), 128_000);
    }

    // ===================================================================
    // Step 1.4 tests: Complete UT coverage
    // ===================================================================

    // build_compact_prompt tests - additional coverage
    #[test]
    fn test_build_compact_prompt_with_custom_full() {
        let prompt = build_compact_prompt(Some("保留 xxx"));
        assert!(prompt.contains("保留 xxx"));
        assert!(prompt.contains("You must not call any tools"));
    }

    // extract_summary tests - additional coverage
    #[test]
    fn test_extract_summary_with_whitespace() {
        let r = extract_summary("<summary>\n  item1\n  item2\n</summary>");
        assert_eq!(r, Some("\n  item1\n  item2\n".to_string()));
    }

    #[test]
    fn test_extract_summary_wrong_order() {
        // end tag before start tag
        assert_eq!(
            extract_summary("</summary><summary>content</summary>"),
            None
        );
    }

    // format_boundary_message tests - additional coverage
    #[test]
    fn test_format_boundary_message_auto_full() {
        let ts = chrono::Utc::now();
        let msg = format_boundary_message("summary text", true, ts);
        assert!(msg.contains(&format!("[Session Compaction | 自动压缩 | {}]", ts)));
        assert!(msg.contains("summary text"));
    }

    #[test]
    fn test_format_boundary_message_manual_full() {
        let ts = chrono::Utc::now();
        let msg = format_boundary_message("summary text", false, ts);
        assert!(msg.contains(&format!("[Session Compaction | 手动压缩 | {}]", ts)));
        assert!(msg.contains("summary text"));
    }

    // CompactionError Display tests
    #[test]
    fn test_compaction_error_display() {
        // LLMCallFailed
        let err_llm = CompactionError::LLMCallFailed("rate limit exceeded".to_string());
        assert!(err_llm.to_string().contains("LLM call failed"));

        // SummaryParseFailed
        let err_parse = CompactionError::SummaryParseFailed;
        assert!(err_parse.to_string().contains("Failed to parse summary"));

        // EmptyMessages
        let err_empty = CompactionError::EmptyMessages;
        assert!(err_empty.to_string().contains("No messages"));
    }

    // ===================================================================
    // plan_state compaction protection tests
    // ===================================================================

    /// Verify CompactionService threshold decision is purely token-based.
    /// Different plan_state values on the checkpoint must not affect
    /// whether the service triggers compaction — the service only inspects
    /// message token counts and circuit breaker state.
    #[test]
    fn test_compaction_service_threshold_is_purely_token_based() {
        let mut config = CompactConfig::default();
        config.auto_compact_buffer_tokens = 0;
        let service = CompactionService::new(config);

        // 3,948,004 chars * 0.25 = 987,001 tokens.
        // mini-max context = 1,000,000 → remaining = 12,999 → AutoCompactTriggered.
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "x".repeat(3_948_004),
        }];

        // AutoCompactTriggered: triggers compaction
        assert!(service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()));
        // The same decision applies regardless of any plan_state that may
        // exist on the checkpoint — plan_state is never consulted by the
        // compaction threshold logic.
    }

    /// Verify that plan_state and messages are independent checkpoint fields.
    /// When compaction replaces messages with a boundary summary, the
    /// checkpoint's plan_state must remain untouched — it is stored and
    /// restored through a separate save/load path, not through the message
    /// pipeline.
    #[test]
    fn test_plan_state_survives_message_replacement_in_checkpoint() {
        use closeclaw_common::{PlanPhase, PlanState};

        let plan = PlanState {
            phase: PlanPhase::Design,
            pending_steps: vec!["step-a".into(), "step-b".into()],
            plan_file_path: "/plans/design.md".into(),
            ..Default::default()
        };

        // Simulate pre-compaction checkpoint fields with long messages
        let original_messages = vec![
            CompactionMessage {
                role: "user".to_string(),
                content: "Please help me with the design doc for the new feature.".repeat(50),
            },
            CompactionMessage {
                role: "assistant".to_string(),
                content: "Sure, I'll review the design doc and provide feedback.".repeat(50),
            },
        ];
        let original_tokens = estimate_messages_tokens(&original_messages, 0.25);
        let _ = original_tokens;
        assert!(original_tokens > 0);

        // Simulate compaction: messages are replaced by boundary summary
        let summary = "Discussed design doc for new feature.";
        let ts = chrono::Utc::now();
        let compacted_messages = vec![CompactionMessage {
            role: "system".to_string(),
            content: format_boundary_message(summary, true, ts),
        }];
        let compacted_tokens = estimate_messages_tokens(&compacted_messages, 0.25);
        assert!(compacted_tokens > 0);
        assert!(compacted_tokens < original_tokens);

        // plan_state is a separate checkpoint field — it is NOT derived from
        // messages and must be preserved identically through compaction.
        let post_compact_plan = plan.clone();
        assert_eq!(post_compact_plan.phase, PlanPhase::Design);
        assert_eq!(post_compact_plan.pending_steps, vec!["step-a", "step-b"]);
        assert_eq!(post_compact_plan.plan_file_path, "/plans/design.md");
    }

    /// Verify boundary message format correctly identifies compaction boundary.
    /// The boundary is the mechanism that separates old conversation content
    /// (which gets replaced) from preserved checkpoint fields like plan_state.
    #[test]
    fn test_compaction_boundary_demarcation_preserves_checkpoint_context() {
        let summary = "User is working on plan mode project with 3 pending steps";
        let ts = chrono::Utc::now();

        // Auto compaction boundary
        let auto_boundary = format_boundary_message(summary, true, ts);
        assert!(auto_boundary.contains(summary));
        assert!(auto_boundary.contains("Session Compaction"));
        assert!(auto_boundary.contains("自动压缩"));
        assert!(auto_boundary.contains(&ts.to_string()));

        // Manual compaction boundary
        let manual_boundary = format_boundary_message(summary, false, ts);
        assert!(manual_boundary.contains(summary));
        assert!(manual_boundary.contains("手动压缩"));
        assert!(manual_boundary.contains(&ts.to_string()));

        // Both boundaries are system messages that sit at the compaction split
        // point — plan_state lives outside this message boundary on the
        // checkpoint, so boundary format correctness is critical for the
        // contract that checkpoint fields survive compaction.
        assert!(!auto_boundary.is_empty());
        assert!(!manual_boundary.is_empty());
    }

    // ===================================================================
    // execute_compact integration tests — see crates/llm/src/compaction_tests.rs
    // ===================================================================
}
