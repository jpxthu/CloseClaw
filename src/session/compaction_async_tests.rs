//! Async tests for compaction module (requires fake-llm feature)

#[cfg(all(test, feature = "fake-llm"))]
mod tests {
    use crate::session::compaction::{execute_compact, CompactionError};
    use closeclaw_llm::fake::FakeProvider;
    use closeclaw_llm::fallback::FallbackClient;
    use closeclaw_llm::LLMRegistry;
    use closeclaw_llm::Message;
    use std::sync::Arc;

    /// Wrap a FakeProvider into an `Arc<dyn Provider>`.
    fn fake_as_dyn(provider: FakeProvider) -> Arc<dyn closeclaw_llm::provider::Provider> {
        Arc::new(provider)
    }

    /// Build a FallbackClient from a FakeProvider for use with `execute_compact`.
    async fn build_fallback_client(provider: FakeProvider) -> FallbackClient {
        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("fake".to_string(), fake_as_dyn(provider))
            .await;
        FallbackClient::from_strings(registry, vec!["fake/fake-model".to_string()])
    }

    #[tokio::test]
    async fn test_execute_compact_success() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Compacted summary content</summary>", "glm-5.1")
            .build();

        let client = build_fallback_client(provider.clone()).await;

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

        let result = execute_compact(&messages, &client, "glm-5.1", None, false).await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.performed);
        assert!(r.boundary_message.contains("Compacted summary content"));
        assert!(r.boundary_message.contains("手动压缩"));
        assert!(r.message.contains("2 messages"));

        // Verify mock received the correct messages
        let captured = provider.captured_internal_requests();
        assert_eq!(captured.len(), 1);
        let req = &captured[0].request;
        // First message is the compaction system prompt
        assert_eq!(req.messages[0].role, "system");
        assert!(req.messages[0].content.contains("session summarizer"));
        // Second message is the user message
        assert_eq!(req.messages[1].role, "user");
        assert_eq!(req.messages[1].content, "Hello, this is a test message");
        // Third message is the assistant message
        assert_eq!(req.messages[2].role, "assistant");
        assert_eq!(req.messages[2].content, "Hi! How can I help you?");
    }

    #[tokio::test]
    async fn test_execute_compact_empty_messages() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>content</summary>", "glm-5.1")
            .build();

        let client = build_fallback_client(provider).await;

        let messages: Vec<Message> = vec![];

        let result = execute_compact(&messages, &client, "glm-5.1", None, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::EmptyMessages)));
    }

    #[tokio::test]
    async fn test_execute_compact_llm_failure() {
        let provider = FakeProvider::builder()
            .then_err(closeclaw_llm::provider::ProviderError::Legacy(
                "rate limit exceeded".to_string(),
            ))
            .build();

        let client = build_fallback_client(provider).await;

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &client, "glm-5.1", None, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::LLMCallFailed(_))));
    }

    #[tokio::test]
    async fn test_execute_compact_no_summary() {
        let provider = FakeProvider::builder()
            .then_ok("No summary tag in response", "glm-5.1")
            .build();

        let client = build_fallback_client(provider).await;

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &client, "glm-5.1", None, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CompactionError::SummaryParseFailed)));
    }

    #[tokio::test]
    async fn test_execute_compact_with_custom_instructions() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Test summary</summary>", "glm-5.1")
            .build();

        let client = build_fallback_client(provider).await;

        let messages = vec![Message {
            role: "user".to_string(),
            content: "Test".to_string(),
        }];

        let result =
            execute_compact(&messages, &client, "glm-5.1", Some("重点保留用户名"), true).await;

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

        let client = build_fallback_client(provider).await;

        let messages = vec![Message {
            role: "user".to_string(),
            content: "test".to_string(),
        }];

        let result = execute_compact(&messages, &client, "glm-5.1", None, true).await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.is_auto);
    }

    #[tokio::test]
    async fn test_execute_compact_filters_system_role() {
        let provider = FakeProvider::builder()
            .then_ok("<summary>Filtered summary</summary>", "glm-5.1")
            .build();

        let client = build_fallback_client(provider.clone()).await;

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: "You are a helpful assistant.".to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "Hello from user".to_string(),
            },
            Message {
                role: "system".to_string(),
                content: "Another system instruction.".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Hello from assistant".to_string(),
            },
        ];

        let result = execute_compact(&messages, &client, "glm-5.1", None, false).await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.performed);

        // Verify the ChatRequest does NOT contain system role messages (except compaction prompt)
        let captured = provider.captured_internal_requests();
        assert_eq!(captured.len(), 1);
        let req = &captured[0].request;

        // Total messages: 1 compaction system prompt + 2 filtered conversation messages = 3
        assert_eq!(req.messages.len(), 3);

        // First is the compaction system prompt
        assert_eq!(req.messages[0].role, "system");
        assert!(req.messages[0].content.contains("session summarizer"));

        // Second is the user message
        assert_eq!(req.messages[1].role, "user");
        assert_eq!(req.messages[1].content, "Hello from user");

        // Third is the assistant message
        assert_eq!(req.messages[2].role, "assistant");
        assert_eq!(req.messages[2].content, "Hello from assistant");
    }
}
