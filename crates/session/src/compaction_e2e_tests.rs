//! End-to-end tests for compaction: async compact tests, config validation, etc.

#[cfg(test)]
mod tests {
    use crate::compaction::{
        ChatFn, CompactConfig, CompactionError, CompactionMessage, CompactionService,
        TokenWarningState,
    };
    use std::sync::Arc;

    // ===================================================================
    // CompactionService::compact tests
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
            result.message.contains("tokens"),
            "message should contain tokens"
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
    // CompactConfig::validate() tests
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
    // token_warning_state .ceil() tests
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

        // Format: "压缩完成：{before} → {after} tokens"
        let expected_format = format!(
            "压缩完成：{} → {} tokens",
            result.before_token_count, result.after_token_count
        );
        assert_eq!(result.message, expected_format);
    }
}
