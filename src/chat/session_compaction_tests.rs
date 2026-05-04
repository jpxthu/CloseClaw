//! Tests for ChatSession compaction integration

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use crate::chat::session::ChatSession;
    use crate::{
        chat::protocol::ServerMessage,
        llm::{LLMRegistry, Message, StubProvider},
        session::compaction::CompactionService,
    };
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::broadcast,
    };

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_manual_compact_basic() {
        use crate::llm::fake::FakeProvider;
        let fake = FakeProvider::builder()
            .then_ok(
                "<summary>recap\n[boundary]</summary>".to_string(),
                "fake-model".to_string(),
            )
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        session.chat_history.push(Message {
            role: "assistant".to_string(),
            content: "hi there".to_string(),
        });
        let json = r#"{"type":"chat.message","content":"/compact","id":"msg1"}"#;
        let msgs: Vec<ServerMessage> = session.handle_line(json).await;
        assert_eq!(msgs.len(), 2);
        let content = match &msgs[0] {
            ServerMessage::ChatResponse { content, .. } => content.clone(),
            _ => panic!("expected ChatResponse"),
        };
        assert!(
            content.contains("压缩成功") || content.contains("Compacted"),
            "expected compaction result, got: {}",
            content
        );
        assert_eq!(session.chat_history.len(), 1);
        assert_eq!(session.chat_history[0].role, "system");
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_manual_compact_with_instructions() {
        use crate::llm::fake::FakeProvider;
        let fake = FakeProvider::builder()
            .then_ok(
                "<summary>recap\n[boundary]</summary>".to_string(),
                "fake-model".to_string(),
            )
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        session.chat_history.push(Message {
            role: "assistant".to_string(),
            content: "hi there".to_string(),
        });
        // Verify that parse_slash_command correctly extracts custom instructions.
        let cmd = crate::mode::slash_command::parse_slash_command("/compact 保留 xxx");
        assert!(cmd.is_some());
        let cmd = cmd.unwrap();
        assert_eq!(cmd.command, "/compact");
        assert_eq!(cmd.args, "保留 xxx");
        // Execute the compact command.
        let json = r#"{"type":"chat.message","content":"/compact 保留 xxx","id":"msg1"}"#;
        let msgs: Vec<ServerMessage> = session.handle_line(json).await;
        assert_eq!(msgs.len(), 2);
        let content = match &msgs[0] {
            ServerMessage::ChatResponse { content, .. } => content.clone(),
            _ => panic!("expected ChatResponse"),
        };
        assert!(
            content.contains("压缩成功") || content.contains("Compacted"),
            "expected compaction result, got: {}",
            content
        );
        assert_eq!(session.chat_history.len(), 1);
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_manual_compact_does_not_add_to_history() {
        use crate::llm::fake::FakeProvider;
        let fake = FakeProvider::builder()
            .then_ok(
                "<summary>recap\n[boundary]</summary>".to_string(),
                "fake-model".to_string(),
            )
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "first".to_string(),
        });
        session.chat_history.push(Message {
            role: "assistant".to_string(),
            content: "first response".to_string(),
        });
        let json = r#"{"type":"chat.message","content":"/compact","id":"msg1"}"#;
        session.handle_line(json).await;
        assert_eq!(session.chat_history.len(), 1);
        assert_eq!(session.chat_history[0].role, "system");
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_manual_compact_failure() {
        use crate::llm::fake::FakeProvider;
        use crate::llm::LLMError;
        let fake = FakeProvider::builder()
            .then_err(LLMError::InvalidRequest("fail".to_string()))
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        let json = r#"{"type":"chat.message","content":"/compact","id":"msg1"}"#;
        let msgs: Vec<ServerMessage> = session.handle_line(json).await;
        assert_eq!(msgs.len(), 2);
        let content = match &msgs[0] {
            ServerMessage::ChatResponse { content, .. } => content.clone(),
            _ => panic!("expected ChatResponse"),
        };
        assert!(
            content.contains("[error]") || content.contains("压缩失败"),
            "expected error, got: {}",
            content
        );
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_auto_compact_triggers_at_threshold() {
        use crate::llm::fake::FakeProvider;
        use crate::session::compaction::CompactConfig;
        let fake = FakeProvider::builder()
            .then_ok(
                "<summary>auto\n[boundary]</summary>".to_string(),
                "fake-model".to_string(),
            )
            .then_ok("normal response".to_string(), "fake-model".to_string())
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.compaction_service = CompactionService::new(CompactConfig {
            chars_per_token: 0.25,
            auto_compact_buffer_tokens: 1000,
            max_consecutive_failures: 3,
        });
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "x".repeat(600_000),
        });
        let json = r#"{"type":"chat.message","content":"hello","id":"msg1"}"#;
        let _msgs = session.handle_line(json).await;
        assert_eq!(session.chat_history.len(), 2);
        assert_eq!(session.chat_history[0].role, "system");
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_auto_compact_not_triggered_below_threshold() {
        use crate::llm::fake::FakeProvider;
        let fake = FakeProvider::builder()
            .then_ok("response".to_string(), "fake-model".to_string())
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        let json = r#"{"type":"chat.message","content":"hi","id":"msg1"}"#;
        let _msgs = session.handle_line(json).await;
        assert!(session.chat_history.len() >= 2);
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_auto_compact_circuit_breaker() {
        use crate::llm::fake::FakeProvider;
        use crate::llm::LLMError;
        use crate::session::compaction::CompactConfig;
        let fake = FakeProvider::builder()
            .then_err(LLMError::InvalidRequest("fail".to_string()))
            .then_ok("normal response".to_string(), "fake-model".to_string())
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.compaction_service = CompactionService::new(CompactConfig {
            chars_per_token: 0.25,
            auto_compact_buffer_tokens: 1000,
            max_consecutive_failures: 3,
        });
        session.compaction_service.record_failure();
        session.compaction_service.record_failure();
        session.compaction_service.record_failure();
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "x".repeat(600_000),
        });
        let json = r#"{"type":"chat.message","content":"test","id":"msg1"}"#;
        let _msgs = session.handle_line(json).await;
        assert!(session.chat_history.len() >= 1);
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_auto_compact_failure_does_not_block() {
        use crate::llm::fake::FakeProvider;
        use crate::llm::LLMError;
        use crate::session::compaction::CompactConfig;
        let fake = FakeProvider::builder()
            .then_err(LLMError::InvalidRequest("compact fail".to_string()))
            .then_ok("normal response".to_string(), "fake-model".to_string())
            .build();
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        session.compaction_service = CompactionService::new(CompactConfig {
            chars_per_token: 0.25,
            auto_compact_buffer_tokens: 1000,
            max_consecutive_failures: 3,
        });
        session.chat_history.push(Message {
            role: "user".to_string(),
            content: "x".repeat(600_000),
        });
        let json = r#"{"type":"chat.message","content":"hello","id":"msg1"}"#;
        let msgs: Vec<ServerMessage> = session.handle_line(json).await;
        assert!(!msgs.is_empty());
    }
}
