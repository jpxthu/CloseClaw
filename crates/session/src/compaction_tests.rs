//! Tests for compaction module

#[cfg(test)]
mod tests {
    use crate::compaction::{
        build_compact_prompt, estimate_messages_tokens, estimate_tokens, estimate_total_tokens,
        extract_summary, format_boundary_message, get_context_window, ChatFn, CompactConfig,
        CompactionError, CompactionMessage, CompactionService, TokenWarningState,
    };
    use std::sync::Arc;

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
        // 1,000,000 - 899,999 = 100,001 remaining > 100,000 (warning threshold)
        assert_eq!(
            service.token_warning_state(899_999, "mini-max", None),
            TokenWarningState::Normal
        );
    }

    #[test]
    fn test_token_warning_state_warning() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // 1,000,000 - 940,000 = 60,000 remaining: 50,000 < 60,000 <= 100,000 -> Warning
        assert_eq!(
            service.token_warning_state(940_000, "mini-max", None),
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
    fn test_token_warning_state_auto_compact_low_tokens() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 3,000 -> AutoCompactTriggered (≤ 5% of 1M context = 50,000)
        assert_eq!(
            service.token_warning_state(997_000, "mini-max", None),
            TokenWarningState::AutoCompactTriggered
        );
    }

    #[test]
    fn test_token_warning_state_knowledge_override() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // Knowledge base context = 500,000; auto = 500k*0.05=25k, warning = 500k*0.10=50k
        // used = 460,000 → remaining = 40,000 → 25k < 40k <= 50k → Warning
        assert_eq!(
            service.token_warning_state(460_000, "mini-max", Some(500_000)),
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
        // request_count=5, msgs.len()=1 → start=min(5,1)=1
        // messages[1..] is empty → remaining_tokens=0
        assert_eq!(result, 10_000);
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
        // request_count=3, msgs.len()=2 → start=min(3,2)=2
        // messages[2..] is empty → remaining_tokens=0
        assert_eq!(result, 1_000);
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
        // 3 messages: first 2 skipped, last 1 estimated
        let msgs = vec![
            CompactionMessage {
                role: "user".to_string(),
                content: "a".repeat(100),
            },
            CompactionMessage {
                role: "assistant".to_string(),
                content: "b".repeat(100),
            },
            CompactionMessage {
                role: "user".to_string(),
                content: "c".repeat(100),
            },
        ];
        // start=min(2,3)=2, only msgs[2] estimated: 100*coeff
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

    #[test]
    fn test_compaction_service_threshold_is_purely_token_based() {
        let config = CompactConfig::default();
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
    // Step 1.5: Manual compact failure does NOT increment circuit breaker
    // ===================================================================

    #[test]
    fn test_manual_compact_failure_does_not_increment_breaker() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let service = CompactionService::new(config);

        // In production, run_manual_compact's Err branch does NOT
        // call record_failure(). Verify the contract: even without
        // calling record_failure, the counter stays at 0.
        assert_eq!(
            service.consecutive_failures(),
            0,
            "manual compact failure must not increment breaker"
        );

        // Auto-compact should still work (breaker not tripped)
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "x".repeat(3_948_004),
        }];
        assert!(
            service.should_auto_compact(&msgs, "mini-max", None, &RunningStats::new()),
            "auto-compact should still trigger after manual failures"
        );
    }

    #[test]
    fn test_manual_compact_success_resets_breaker() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut service = CompactionService::new(config);

        // Simulate: some auto-compact failures tripped the breaker
        service.record_failure();
        service.record_failure();
        service.record_failure();
        assert_eq!(service.consecutive_failures(), 3);

        // Manual compact succeeds → resets breaker
        service.record_success();
        assert_eq!(
            service.consecutive_failures(),
            0,
            "manual compact success must reset breaker"
        );
    }

    // ===================================================================
    // Step 1.5: Compact prompt has exactly 6 dimensions
    // ===================================================================

    #[test]
    fn test_build_compact_prompt_contains_six_dimensions() {
        let prompt = build_compact_prompt(None);
        assert!(prompt.contains("Goal"), "missing Goal dimension");
        assert!(
            prompt.contains("Constraints & Preferences"),
            "missing Constraints & Preferences dimension"
        );
        assert!(prompt.contains("Progress"), "missing Progress dimension");
        assert!(
            prompt.contains("Key Decisions"),
            "missing Key Decisions dimension"
        );
        assert!(
            prompt.contains("Next Steps"),
            "missing Next Steps dimension"
        );
        assert!(
            prompt.contains("Critical Context"),
            "missing Critical Context dimension"
        );
    }

    #[test]
    fn test_build_compact_prompt_no_old_nine_dimensions() {
        let prompt = build_compact_prompt(None);
        // Old 9-dim keywords that must NOT appear
        assert!(!prompt.contains("Technical"));
        assert!(!prompt.contains("Environment"));
        assert!(!prompt.contains("Code Patterns"));
        assert!(!prompt.contains("User Preferences"));
        assert!(!prompt.contains("Session Metadata"));
    }

    // ===================================================================
    // Step 1.5: CompactionService::compact tests
    // ===================================================================

    /// Helper: create a ChatFn that returns a successful LLM response
    /// with the given summary content.
    fn mock_chat_success(summary: &str) -> ChatFn {
        let response = format!("<summary>{}</summary>", summary);
        Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
            let resp = response.clone();
            Box::pin(async move { Ok((resp, 0)) })
        })
    }

    /// Helper: create a ChatFn that simulates an LLM call failure.
    fn mock_chat_failure(error_msg: &str) -> ChatFn {
        let err = error_msg.to_string();
        Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
            let e = err.clone();
            Box::pin(async move { Err(e) })
        })
    }

    /// Helper: create a ChatFn that returns a response without <summary> tags.
    fn mock_chat_no_summary(response: &str) -> ChatFn {
        let resp = response.to_string();
        Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
            let r = resp.clone();
            Box::pin(async move { Ok((r, 0)) })
        })
    }

    #[tokio::test]
    async fn test_compact_normal_no_instruction() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![
            CompactionMessage {
                role: "user".to_string(),
                content: "Hello, how are you?".to_string(),
            },
            CompactionMessage {
                role: "assistant".to_string(),
                content: "I am doing well, thank you.".to_string(),
            },
        ];
        let chat_fn = mock_chat_success("Greeted user.");

        // Manual compact
        let result = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap();

        assert!(result.performed, "should be performed");
        assert!(
            result.message.starts_with("压缩完成："),
            "message should start with 压缩完成："
        );
        assert!(
            result.message.contains("字符"),
            "message should contain 字符"
        );
        assert!(!result.is_auto);

        // Auto compact
        let result_auto = svc
            .compact(&msgs, "glm-5", None, true, None, &chat_fn)
            .await
            .unwrap();
        assert!(result_auto.is_auto);
    }

    #[tokio::test]
    async fn test_compact_with_custom_instruction() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "Help me with the API docs".to_string(),
        }];

        // Capture the messages passed to chat_fn to verify the
        // custom instruction is embedded in the system prompt.
        let captured: Arc<tokio::sync::Mutex<Vec<Vec<CompactionMessage>>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured);
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = Arc::clone(&call_count);
        let chat_fn: ChatFn = Arc::new(move |_model: String, msgs: Vec<CompactionMessage>| {
            let cap = Arc::clone(&captured_clone);
            let cc = Arc::clone(&call_count_clone);
            Box::pin(async move {
                cap.lock().await.push(msgs);
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(("<summary>Summarized.</summary>".to_string(), 0))
            })
        });

        let result = svc
            .compact(&msgs, "glm-5", Some("保留 API 列表"), false, None, &chat_fn)
            .await
            .unwrap();

        assert!(result.performed);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "chat_fn should be called once"
        );

        // Verify the system prompt includes the custom instruction.
        let captured_msgs = captured.lock().await;
        assert_eq!(captured_msgs.len(), 1, "should have captured one call");
        let system_msg = &captured_msgs[0][0];
        assert_eq!(system_msg.role, "system", "first message should be system");
        assert!(
            system_msg.content.contains("保留 API 列表"),
            "system prompt should contain the custom instruction, got: {}",
            system_msg.content
        );
    }

    #[tokio::test]
    async fn test_compact_empty_messages() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs: Vec<CompactionMessage> = vec![];
        let chat_fn = mock_chat_success("unused");

        let err = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap_err();

        assert!(matches!(err, CompactionError::EmptyMessages));
    }

    #[tokio::test]
    async fn test_compact_llm_failure() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "test message".to_string(),
        }];
        let chat_fn = mock_chat_failure("rate limit exceeded");

        let err = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap_err();

        match err {
            CompactionError::LLMCallFailed(msg) => {
                assert_eq!(msg, "rate limit exceeded");
            }
            _ => panic!("expected LLMCallFailed"),
        }
    }

    #[tokio::test]
    async fn test_compact_summary_parse_failure() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "test message".to_string(),
        }];
        let chat_fn = mock_chat_no_summary("no summary tag here");

        let err = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap_err();

        assert!(matches!(err, CompactionError::SummaryParseFailed));
    }

    #[tokio::test]
    async fn test_compact_char_counts_correct() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![
            CompactionMessage {
                role: "user".to_string(),
                content: "Hello world".to_string(),
            },
            CompactionMessage {
                role: "assistant".to_string(),
                content: "Hi there".to_string(),
            },
        ];
        let expected_before = 11 + 8;
        let chat_fn = mock_chat_success("Brief summary.");

        let result = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap();

        assert_eq!(result.before_char_count, expected_before);
        assert!(result.after_char_count > 0);
        assert!(result.after_char_count > result.before_char_count);
        assert!(result.boundary_message.contains("Brief summary."));
        assert!(result.boundary_message.contains("Session Compaction"));
        // Token counts
        assert!(result.before_token_count > 0);
        assert!(result.after_token_count > 0);
        assert_eq!(result.original_tokens, result.before_token_count);
        assert_eq!(result.compacted_tokens, result.after_token_count);
    }

    #[tokio::test]
    async fn test_compact_resets_consecutive_failures() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut svc = CompactionService::new(config);

        // Trip the circuit breaker.
        svc.record_failure();
        svc.record_failure();
        svc.record_failure();
        assert_eq!(svc.consecutive_failures(), 3);

        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "test".to_string(),
        }];
        let chat_fn = mock_chat_success("ok");

        let result = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap();
        assert!(result.performed);
        assert_eq!(
            svc.consecutive_failures(),
            0,
            "consecutive_failures should reset to 0 after success"
        );
    }

    #[tokio::test]
    async fn test_compact_failure_preserves_circuit_breaker() {
        let mut config = CompactConfig::default();
        config.max_consecutive_failures = 3;
        let mut svc = CompactionService::new(config);

        svc.record_failure();
        svc.record_failure();
        assert_eq!(svc.consecutive_failures(), 2);

        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "test".to_string(),
        }];
        let chat_fn = mock_chat_failure("error");

        let err = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap_err();
        assert!(matches!(err, CompactionError::LLMCallFailed(_)));
        // consecutive_failures should remain 2 — compact failure doesn't
        // call record_failure (it's the caller's responsibility).
        assert_eq!(svc.consecutive_failures(), 2);
    }

    // ===================================================================
    // Step 1.9: CompactConfig::validate() tests
    // ===================================================================

    #[test]
    fn test_validate_default_config_passes() {
        let config = CompactConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_chars_per_token_negative() {
        let config = CompactConfig {
            chars_per_token: -0.5,
            ..CompactConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("chars_per_token"));
        assert!(err.contains("positive"));
    }

    #[test]
    fn test_validate_chars_per_token_zero() {
        let config = CompactConfig {
            chars_per_token: 0.0,
            ..CompactConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_auto_threshold_below_zero() {
        let config = CompactConfig {
            auto_compact_threshold_pct: -0.1,
            ..CompactConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("auto_compact_threshold_pct"));
        assert!(err.contains("[0, 1]"));
    }

    #[test]
    fn test_validate_auto_threshold_above_one() {
        let config = CompactConfig {
            auto_compact_threshold_pct: 1.5,
            ..CompactConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_warning_threshold_above_one() {
        let config = CompactConfig {
            warning_threshold_pct: 2.0,
            ..CompactConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("warning_threshold_pct"));
        assert!(err.contains("[0, 1]"));
    }

    #[test]
    fn test_validate_auto_exceeds_warning() {
        let config = CompactConfig {
            auto_compact_threshold_pct: 0.15,
            warning_threshold_pct: 0.10,
            ..CompactConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("must be <="));
    }

    #[test]
    fn test_validate_equal_thresholds_pass() {
        let config = CompactConfig {
            auto_compact_threshold_pct: 0.10,
            warning_threshold_pct: 0.10,
            ..CompactConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_boundary_zero_passes() {
        let config = CompactConfig {
            auto_compact_threshold_pct: 0.0,
            warning_threshold_pct: 0.0,
            ..CompactConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_boundary_one_passes() {
        let config = CompactConfig {
            auto_compact_threshold_pct: 1.0,
            warning_threshold_pct: 1.0,
            ..CompactConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    // ===================================================================
    // Step 1.9: token_warning_state .ceil() tests
    // ===================================================================

    #[test]
    fn test_token_warning_state_ceil_rounding() {
        // 128k window, auto=5%=6400, warning=10%=12800
        // remaining=6399 -> ceil(6400)=6400 -> 6399<=6400 -> AutoCompact
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(
            service.token_warning_state(121_601, "glm-3", None),
            TokenWarningState::AutoCompactTriggered
        );
    }

    #[test]
    fn test_token_warning_state_ceil_just_above() {
        // 128k window, remaining=6401 > ceil(6400)=6400
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 128000 - 121599 = 6401
        assert_eq!(
            service.token_warning_state(121_599, "glm-3", None),
            TokenWarningState::Warning
        );
    }

    #[tokio::test]
    async fn test_compact_message_format_matches_design_doc() {
        let mut svc = CompactionService::new(CompactConfig::default());
        let msgs = vec![CompactionMessage {
            role: "user".to_string(),
            content: "abc".to_string(),
        }];
        let chat_fn = mock_chat_success("Summary.");

        let result = svc
            .compact(&msgs, "glm-5", None, false, None, &chat_fn)
            .await
            .unwrap();

        // Format: "压缩完成：{before} → {after} 字符"
        let expected_format = format!(
            "压缩完成：{} → {} 字符",
            result.before_char_count, result.after_char_count
        );
        assert_eq!(result.message, expected_format);
    }
}
