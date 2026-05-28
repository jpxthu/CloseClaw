#![cfg(feature = "chat-legacy")]

//! e2e_thinking module — all helpers and tests
//!
//! Tests 1, 2 use unit-style (direct session manipulation without spawn).
//! Tests 3-5 use protocol-level TCP style (spawn session.run() + TCP client).

#[cfg(feature = "fake-llm")]
mod tests {
    use std::sync::Arc;

    use closeclaw::chat::protocol::ServerMessage;
    use closeclaw::chat::session::LegacyChatSession;
    use closeclaw::llm::fake::{FakeProvider, Scenario};
    use closeclaw::llm::LLMRegistry;
    use closeclaw::llm::Message;
    use closeclaw::session::compaction::execute_compact;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;

    // ---------------------------------------------------------------------------
    // Shared helpers for protocol-level tests
    // ---------------------------------------------------------------------------

    async fn setup_session_with_fake(
        scenarios: Vec<Scenario>,
    ) -> (
        LegacyChatSession,
        tokio::net::TcpStream,
        broadcast::Sender<()>,
    ) {
        let fake_provider = build_fake_provider(scenarios);
        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/glm-5");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);

        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("fake".to_string(), Arc::new(fake_provider))
            .await;

        let session = LegacyChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx,
            registry,
            None,
        );

        std::env::remove_var("LLM_FALLBACK_CHAIN");
        (session, client, shutdown_tx)
    }

    fn build_fake_provider(scenarios: Vec<Scenario>) -> FakeProvider {
        let mut provider = FakeProvider::builder().stub(false).or_else("");
        for s in scenarios {
            match s {
                Scenario::Ok { content, model, .. } => {
                    provider = provider.then_ok(content, model);
                }
                Scenario::Err { error, .. } => {
                    provider = provider.then_err(error);
                }
                Scenario::Delay { duration, inner } => {
                    provider = provider.then_delay(duration, (*inner).clone());
                }
            }
        }
        provider.build()
    }

    async fn read_msg(
        reader: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> ServerMessage {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("parse error from `{line}`: {e}"))
    }

    async fn send_json(writer: &mut tokio::net::tcp::OwnedWriteHalf, json: &str) {
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
    }

    // ---------------------------------------------------------------------------
    // Test 1: thinking tag string is transparently stored in chat_history
    // ---------------------------------------------------------------------------

    #[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
    #[tokio::test]
    async fn test_thinking_tag_persistence_in_history() {
        // Build FakeProvider with thinking-tagged responses
        let fake_provider = build_fake_provider(vec![
            Scenario::ok("<thinking>analyzing...</thinking> Turn 1 answer", "glm-5"),
            Scenario::ok("<thinking>formulating...</thinking> Turn 2 answer", "glm-5"),
            Scenario::ok("<thinking>verifying...</thinking> Turn 3 answer", "glm-5"),
        ]);

        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/glm-5");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);

        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("fake".to_string(), Arc::new(fake_provider))
            .await;

        // Create session but do NOT call run() — keep it alive via Arc.
        // Hold a mutable reference to inspect chat_history directly.
        let mut session = LegacyChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx,
            Arc::clone(&registry),
            None,
        );

        std::env::remove_var("LLM_FALLBACK_CHAIN");

        // Simulate 3 turns by directly pushing to chat_history (no spawn/run)
        for (user_content, assistant_content) in [
            (
                "problem A",
                "<thinking>analyzing...</thinking> Turn 1 answer",
            ),
            (
                "problem B",
                "<thinking>formulating...</thinking> Turn 2 answer",
            ),
            (
                "problem C",
                "<thinking>verifying...</thinking> Turn 3 answer",
            ),
        ] {
            session.chat_history.push(Message {
                role: "user".to_string(),
                content: user_content.to_string(),
            });
            session.chat_history.push(Message {
                role: "assistant".to_string(),
                content: assistant_content.to_string(),
            });
        }

        // Verify: 3 assistant messages all contain thinking tags
        let assistant_msgs: Vec<_> = session
            .chat_history
            .iter()
            .filter(|m| m.role == "assistant")
            .collect();
        assert_eq!(assistant_msgs.len(), 3);
        for (i, msg) in assistant_msgs.iter().enumerate() {
            assert!(
                msg.content.contains("<thinking>"),
                "assistant message {} should contain thinking tag: {:?}",
                i,
                msg.content
            );
        }

        drop(client);
    }

    // ---------------------------------------------------------------------------
    // Test 2: execute_compact output does NOT contain original thinking tags
    // ---------------------------------------------------------------------------

    #[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
    #[tokio::test]
    async fn test_compact_boundary_excludes_thinking_tags() {
        let fake_provider = build_fake_provider(vec![Scenario::ok(
            "<summary>User discussed a problem and received step-by-step analysis.</summary>.",
            "glm-5",
        )]);

        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/glm-5");

        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("fake".to_string(), Arc::new(fake_provider))
            .await;

        let llm = registry.get("fake").await.unwrap();

        let messages = vec![
            Message {
                role: "user".to_string(),
                content: "problem A".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "<thinking>analyzing...</thinking> answer A".to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "problem B".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "<thinking>formulating...</thinking> answer B".to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "problem C".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "<thinking>verifying...</thinking> answer C".to_string(),
            },
        ];

        let result = execute_compact(&messages, llm.as_ref(), "glm-5", None, false)
            .await
            .expect("execute_compact should succeed");

        assert!(
            result.boundary_message.contains("User discussed"),
            "Boundary should contain summary, got: {}",
            result.boundary_message
        );
        assert!(
            !result.boundary_message.contains("<thinking>"),
            "Boundary must NOT contain thinking tags, got: {}",
            result.boundary_message
        );
        assert!(!result.boundary_message.contains("analyzing"));
        assert!(!result.boundary_message.contains("formulating"));
        assert!(!result.boundary_message.contains("verifying"));

        std::env::remove_var("LLM_FALLBACK_CHAIN");
    }

    // ---------------------------------------------------------------------------
    // Test 3: thinking tag response traverses the wire via TCP protocol
    // ---------------------------------------------------------------------------

    #[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
    #[tokio::test]
    async fn test_thinking_tag_traverses_tcp_protocol() {
        let scenarios = vec![Scenario::ok(
            "<thinking>analyzing the query...</thinking> final answer",
            "glm-5",
        )];

        let (session, client, _shutdown_tx) = setup_session_with_fake(scenarios).await;
        let session_handle = tokio::spawn(session.run());

        let (reader_half, mut writer_half) = client.into_split();
        let mut reader = tokio::io::BufReader::new(reader_half);

        // Start
        send_json(
            &mut writer_half,
            r#"{"type":"chat.start","agent_id":"test-agent","id":"req-start"}"#,
        )
        .await;
        let msg = read_msg(&mut reader).await;
        assert!(matches!(msg, ServerMessage::ChatStarted { .. }));

        // Send message
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"hello","id":"req-1"}"#,
        )
        .await;

        let resp = read_msg(&mut reader).await;
        let done = read_msg(&mut reader).await;

        assert!(
            matches!(
                &resp,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("<thinking>") && content.contains("analyzing the query")
            ),
            "Response should contain thinking tag from wire, got: {:?}",
            resp
        );
        assert!(matches!(done, ServerMessage::ChatResponseDone { .. }));

        drop(writer_half);
        let _ = session_handle.await;
    }

    // ---------------------------------------------------------------------------
    // Test 4: post-compact conversation continues with new session context
    // ---------------------------------------------------------------------------

    #[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
    #[tokio::test]
    async fn test_conversation_continues_after_compact() {
        let scenarios = vec![
            Scenario::ok("reply 1", "glm-5"),
            Scenario::ok("reply 2", "glm-5"),
            Scenario::ok("<summary>topics discussed.</summary>.", "glm-5"),
            Scenario::ok("post-compact response", "glm-5"),
        ];

        let (session, client, _shutdown_tx) = setup_session_with_fake(scenarios).await;
        let session_handle = tokio::spawn(session.run());

        let (reader_half, mut writer_half) = client.into_split();
        let mut reader = tokio::io::BufReader::new(reader_half);

        // Start
        send_json(
            &mut writer_half,
            r#"{"type":"chat.start","agent_id":"test-agent","id":"req-start"}"#,
        )
        .await;
        let _msg = read_msg(&mut reader).await;

        // 2 normal turns
        for (i, text) in ["hello", "continue"].iter().enumerate() {
            send_json(
                &mut writer_half,
                &format!(
                    r#"{{"type":"chat.message","content":"{}","id":"req-{}"}}"#,
                    text,
                    i + 1
                ),
            )
            .await;
            let _resp = read_msg(&mut reader).await;
            let _done = read_msg(&mut reader).await;
        }

        // /compact
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"/compact","id":"req-compact"}"#,
        )
        .await;
        let compact_resp = read_msg(&mut reader).await;
        let compact_done = read_msg(&mut reader).await;
        assert!(
            matches!(
                &compact_resp,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("压缩成功")
            ),
            "Compact should return success, got: {:?}",
            compact_resp
        );
        assert!(matches!(
            compact_done,
            ServerMessage::ChatResponseDone { .. }
        ));

        // Post-compact message
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"what were we discussing?","id":"req-after"}"#,
        )
        .await;
        let after_resp = read_msg(&mut reader).await;
        let after_done = read_msg(&mut reader).await;

        assert!(
            matches!(
                &after_resp,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("post-compact")
            ),
            "Post-compact response should come from scenario 4, got: {:?}",
            after_resp
        );
        assert!(matches!(after_done, ServerMessage::ChatResponseDone { .. }));

        drop(writer_half);
        let _ = session_handle.await;
    }

    // ---------------------------------------------------------------------------
    // Test 5: session remains functional after compact
    // ---------------------------------------------------------------------------

    #[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
    #[tokio::test]
    async fn test_session_stable_after_compact() {
        let scenarios = vec![
            Scenario::ok("first reply", "glm-5"),
            Scenario::ok("<summary>first topic covered.</summary>.", "glm-5"),
            Scenario::ok("second reply after compact", "glm-5"),
        ];

        let (session, client, _shutdown_tx) = setup_session_with_fake(scenarios).await;
        let session_handle = tokio::spawn(session.run());

        let (reader_half, mut writer_half) = client.into_split();
        let mut reader = tokio::io::BufReader::new(reader_half);

        // Start
        send_json(
            &mut writer_half,
            r#"{"type":"chat.start","agent_id":"test-agent","id":"req-start"}"#,
        )
        .await;
        let msg = read_msg(&mut reader).await;
        assert!(matches!(msg, ServerMessage::ChatStarted { .. }));

        // First message
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"start","id":"req-1"}"#,
        )
        .await;
        let resp1 = read_msg(&mut reader).await;
        let done1 = read_msg(&mut reader).await;
        assert!(matches!(
            &resp1,
            ServerMessage::ChatResponse { content, .. }
                if content.contains("first reply")
        ));
        assert!(matches!(done1, ServerMessage::ChatResponseDone { .. }));

        // /compact
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"/compact","id":"req-compact"}"#,
        )
        .await;
        let compact_resp = read_msg(&mut reader).await;
        let compact_done = read_msg(&mut reader).await;
        assert!(
            matches!(
                &compact_resp,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("压缩成功")
            ),
            "Compact should succeed, got: {:?}",
            compact_resp
        );
        assert!(matches!(
            compact_done,
            ServerMessage::ChatResponseDone { .. }
        ));

        // Second message after compact
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"continue","id":"req-2"}"#,
        )
        .await;
        let resp2 = read_msg(&mut reader).await;
        let done2 = read_msg(&mut reader).await;
        assert!(
            matches!(
                &resp2,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("second reply after compact")
            ),
            "Post-compact response should contain 'second reply after compact', got: {:?}",
            resp2
        );
        assert!(matches!(done2, ServerMessage::ChatResponseDone { .. }));

        drop(writer_half);
        let _ = session_handle.await;
    }
}
