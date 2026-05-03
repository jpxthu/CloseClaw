//! Tests for compaction module

#[cfg(test)]
mod tests {
    use crate::llm::Message;
    use crate::session::compaction::{
        build_compact_prompt, estimate_messages_tokens, estimate_tokens, extract_summary,
        format_boundary_message, get_context_window, CompactConfig, CompactionError,
        CompactionService, TokenWarningState,
    };

    #[test]
    fn test_estimate_tokens_english() {
        // "hello" = 5 chars * 0.25 = 1.25 -> ceil = 2
        let tokens = estimate_tokens("hello");
        assert!(tokens >= 2 && tokens <= 5, "expected 2-5, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_chinese() {
        // "你好" = 2 chars * 0.25 = 0.5 -> ceil = 1
        let tokens = estimate_tokens("你好");
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_emoji() {
        // "🎉🎊🔥" = 3 chars * 0.25 = 0.75 -> ceil = 1
        let tokens = estimate_tokens("🎉🎊🔥");
        assert!(tokens >= 1 && tokens <= 4, "expected 1-4, got {}", tokens);
    }

    #[test]
    fn test_estimate_tokens_long_string() {
        let s = "a".repeat(1000);
        assert_eq!(estimate_tokens(&s), 250);
    }

    #[test]
    fn test_get_context_window_minimax() {
        assert_eq!(get_context_window("mini-max"), 1_000_000);
    }

    #[test]
    fn test_get_context_window_glm() {
        assert_eq!(get_context_window("glm-5.1"), 256_000);
    }

    #[test]
    fn test_get_context_window_unknown() {
        assert_eq!(get_context_window("unknown-model-xyz"), 128_000);
    }

    #[test]
    fn test_should_auto_compact_below_threshold() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "short".to_string(),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max"));
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
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "x".repeat(900_000),
        }];
        // Circuit breaker should trip
        assert!(!service.should_auto_compact(&msgs, "mini-max"));
    }

    #[test]
    fn test_token_warning_state_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // 1,000,000 - 50,000 = 950,000 used -> 50,000 remaining > 20,000
        assert_eq!(
            service.token_warning_state(950_000, "mini-max"),
            TokenWarningState::Normal
        );
    }

    #[test]
    fn test_token_warning_state_warning() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 20,000 -> Warning
        assert_eq!(
            service.token_warning_state(980_000, "mini-max"),
            TokenWarningState::Warning
        );
    }

    #[test]
    fn test_token_warning_state_auto_compact() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 13,000 -> AutoCompactTriggered
        assert_eq!(
            service.token_warning_state(987_000, "mini-max"),
            TokenWarningState::AutoCompactTriggered
        );
    }

    #[test]
    fn test_token_warning_state_blocking() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        // remaining = 3,000 -> Blocking
        assert_eq!(
            service.token_warning_state(997_000, "mini-max"),
            TokenWarningState::Blocking
        );
    }

    #[test]
    fn test_percent_left_normal() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(500_000, "mini-max"), 50);
    }

    #[test]
    fn test_percent_left_zero_used() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(0, "mini-max"), 100);
    }

    #[test]
    fn test_percent_left_near_full() {
        let config = CompactConfig::default();
        let service = CompactionService::new(config);
        assert_eq!(service.percent_left(999_000, "mini-max"), 0);
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
        let msgs = vec![Message {
            role: "user".to_string(),
            content: "x".repeat(4_000_000),
        }];
        assert!(!service.should_auto_compact(&msgs, "mini-max"));
        service.record_success();
        assert!(service.should_auto_compact(&msgs, "mini-max"));
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
        let msg = format_boundary_message("summary", true);
        assert!(msg.contains("自动压缩"));
    }

    #[test]
    fn test_format_boundary_message_manual() {
        let msg = format_boundary_message("summary", false);
        assert!(msg.contains("手动压缩"));
    }

    // Step 1.4 tests: Complete UT coverage

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
        let msg = format_boundary_message("summary text", true);
        assert!(msg.contains("[Session Compaction | 自动压缩]"));
        assert!(msg.contains("summary text"));
    }

    #[test]
    fn test_format_boundary_message_manual_full() {
        let msg = format_boundary_message("summary text", false);
        assert!(msg.contains("[Session Compaction | 手动压缩]"));
        assert!(msg.contains("summary text"));
    }

    // CompactionError Display tests
    #[test]
    fn test_compaction_error_display() {
        // LLMCallFailed
        let err_llm = CompactionError::LLMCallFailed(crate::llm::LLMError::RateLimitExceeded);
        assert!(err_llm.to_string().contains("LLM call failed"));

        // SummaryParseFailed
        let err_parse = CompactionError::SummaryParseFailed;
        assert!(err_parse.to_string().contains("Failed to parse summary"));

        // EmptyMessages
        let err_empty = CompactionError::EmptyMessages;
        assert!(err_empty.to_string().contains("No messages"));
    }
}

#[cfg(all(test, feature = "fake-llm"))]
mod async_tests {
    use crate::llm::fake::{FakeProvider, Scenario};
    use crate::llm::Message;
    use crate::session::compaction::{execute_compact, CompactionError, CompactionResult};

    #[tokio::test]
    async fn test_execute_compact_success() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Compacted summary content</summary>", "glm-5.1")
            .build();

        let messages = vec![
            Message {
                role: "user".to_string(),
                content: "Hello, this is a test message".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Hi! How can I help you?".to_string(),
            },
        ];

        let result = execute_compact(&messages, &provider, "glm-5.1", None, false).await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.performed);
        assert!(r.boundary_message.contains("Compacted summary content"));
        assert!(r.boundary_message.contains("手动压缩"));
        assert!(r.message.contains("2 messages"));
    }

    #[tokio::test]
    async fn test_execute_compact_empty_messages() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>content</summary>", "glm-5.1")
            .build();

        let messages: Vec<Message> = vec![];

        let result = execute_compact(&messages, &provider, "glm-5.1", None, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::EmptyMessages)));
    }

    #[tokio::test]
    async fn test_execute_compact_llm_failure() {
        let provider = FakeProvider::builder()
            .then_err(crate::llm::LLMError::RateLimitExceeded)
            .build();

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &provider, "glm-5.1", None, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::LLMCallFailed(_))));
    }

    #[tokio::test]
    async fn test_execute_compact_no_summary() {
        let provider = FakeProvider::builder()
            .then_ok("No summary tag in response", "glm-5.1")
            .build();

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &provider, "glm-5.1", None, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::SummaryParseFailed)));
    }

    #[tokio::test]
    async fn test_execute_compact_with_custom_instructions() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Test summary</summary>", "glm-5.1")
            .build();

        let messages = vec![Message {
            role: "user".to_string(),
            content: "Test".to_string(),
        }];

        let result = execute_compact(
            &messages,
            &provider,
            "glm-5.1",
            Some("重点保留用户名"),
            true,
        )
        .await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.boundary_message.contains("Test summary"));
        assert!(r.boundary_message.contains("自动压缩"));
    }

    #[tokio::test]
    async fn test_execute_compact_auto_trigger() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Auto summary</summary>", "glm-5.1")
            .build();

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &provider, "glm-5.1", None, true).await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.is_auto);
    }
}
