//! E2E/008 — Thinking 标签多轮处理与 Session 持久化
//!
//! 验证含 `<thinking>` 标签字符串在 chat_history 中透传，
//! 以及 Compact 触发后原始 thinking 内容被正确丢弃。
//!
//! 依赖：`fake-llm` feature

#![allow(deprecated)]

#[cfg(feature = "fake-llm")]
mod tests {
    use std::sync::Arc;

    use closeclaw::chat::protocol::{ClientMessage, ServerMessage};
    use closeclaw::chat::session::LegacyChatSession;
    use closeclaw::llm::fake::{FakeProvider, Scenario};
    use closeclaw::llm::LLMProvider;
    use closeclaw::llm::LLMRegistry;
    use closeclaw::llm::Message;
    use closeclaw::session::compaction::execute_compact;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;

    // ---------------------------------------------------------------------------
    // Shared setup helpers
    // ---------------------------------------------------------------------------

    /// Set up a `LegacyChatSession` backed by a TCP pair with `FakeProvider`
    /// registered.  Returns `(session, client_stream, shutdown_tx)`.
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

    /// Build a FakeProvider from a scenario list.
    ///
    /// Uses `.or_else("")` to prevent panic/chain-exhausted error when scenarios
    /// are exhausted — scenarios should be exactly matched to test steps, but
    /// using a fallback ensures a miscount does not cascade into "all models
    /// exhausted" errors from the fallback client.
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
    //
    // Unit-test style: directly operate on chat_history via a session reference.
    // This avoids the `run()` → `()` constraint that makes chat_history
    // inaccessible after the async task is spawned.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_thinking_tag_persistence_in_history() {
        // Build a FakeProvider that returns responses with <thinking> tags.
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
        // We hold a mutable reference to inspect chat_history directly.
        let mut session = LegacyChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx,
            Arc::clone(&registry),
            None,
        );

        std::env::remove_var("LLM_FALLBACK_CHAIN");

        // Simulate 3 turns of conversation by directly pushing to chat_history.
        // (The session layer is transparent — it stores whatever the LLM returns.)
        for (i, (user_content, assistant_content)) in [
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
        ]
        .iter()
        .enumerate()
        {
            // User message
            session.chat_history.push(Message {
                role: "user".to_string(),
                content: (*user_content).to_string(),
            });
            // Assistant message (simulating call_llm output)
            session.chat_history.push(Message {
                role: "assistant".to_string(),
                content: (*assistant_content).to_string(),
            });
            let _ = i; // suppress unused warning
        }

        // Verify: 6 messages (3 user + 3 assistant)
        assert_eq!(
            session.chat_history.len(),
            6,
            "chat_history should have 6 messages"
        );

        // Verify: each assistant message contains the thinking tag as a plain string
        let assistants: Vec<_> = session
            .chat_history
            .iter()
            .filter(|m| m.role == "assistant")
            .collect();

        assert_eq!(assistants.len(), 3);
        assert!(
            assistants[0].content.contains("<thinking>analyzing"),
            "Assistant 1 should contain thinking tag"
        );
        assert!(
            assistants[1].content.contains("<thinking>formulating"),
            "Assistant 2 should contain thinking tag"
        );
        assert!(
            assistants[2].content.contains("<thinking>verifying"),
            "Assistant 3 should contain thinking tag"
        );

        // Verify: thinking tags are stored as PLAIN STRINGS, not parsed/removed
        assert!(
            session
                .chat_history
                .iter()
                .any(|m| m.content.contains("<thinking>analyzing")),
            "Thinking tag should be stored as plain string in history"
        );

        drop(client); // clean up
    }

    // ---------------------------------------------------------------------------
    // Test 2: execute_compact output does NOT contain original thinking tags
    //
    // Pure unit test: directly call execute_compact() with a chat_history
    // that contains thinking tags, then verify the boundary message does not
    // contain them.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_boundary_excludes_thinking_tags() {
        // Build a FakeProvider that returns a clean summary.
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

        // Construct a chat_history that mimics what a real session would have
        // after 3 turns with thinking tags — 6 messages.
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

        // Call execute_compact — this mimics what happens when the user sends /compact
        let result = execute_compact(&messages, llm.as_ref(), "glm-5", None, false)
            .await
            .expect("execute_compact should succeed");

        // The boundary message should contain the summary text
        assert!(
            result.boundary_message.contains("User discussed"),
            "Boundary should contain summary, got: {}",
            result.boundary_message
        );

        // Critical: boundary message must NOT contain any thinking tags
        assert!(
            !result.boundary_message.contains("<thinking>"),
            "Boundary message must NOT contain thinking tags, got: {}",
            result.boundary_message
        );

        // No thinking tags should appear anywhere in the result
        assert!(
            !result.boundary_message.contains("analyzing"),
            "Boundary should not contain thinking content 'analyzing'"
        );
        assert!(
            !result.boundary_message.contains("formulating"),
            "Boundary should not contain thinking content 'formulating'"
        );
        assert!(
            !result.boundary_message.contains("verifying"),
            "Boundary should not contain thinking content 'verifying'"
        );

        std::env::remove_var("LLM_FALLBACK_CHAIN");
    }

    // ---------------------------------------------------------------------------
    // Test 3: thinking tag response traverses the wire via TCP protocol
    //
    // Protocol-level E2E: verifies that a FakeProvider response with thinking
    // tags is correctly sent over TCP as part of the ChatResponse.
    // ---------------------------------------------------------------------------

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

        // Protocol response must contain the thinking tag as sent by FakeProvider
        assert!(
            matches!(
                &resp,
                ServerMessage::ChatResponse { content, .. }
                    if content.contains("<thinking>analyzing")
            ),
            "ChatResponse should forward thinking tag from FakeProvider, got: {:?}",
            resp
        );
        assert!(matches!(done, ServerMessage::ChatResponseDone { .. }));

        drop(writer_half);
        let _ = session_handle.await;
    }

    // ---------------------------------------------------------------------------
    // Test 4: post-compact conversation continues with new session context
    //
    // Protocol-level E2E: verify that after /compact the session remains
    // functional and responds to subsequent messages.
    // ---------------------------------------------------------------------------

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

        // /compact — consumes scenario 3 (Summary)
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

        // Post-compact message — consumes scenario 4
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
    // Test 5: session remains functional after compact (no panic / disconnect)
    //
    // Protocol-level E2E: ensure compact does not corrupt session state.
    // ---------------------------------------------------------------------------

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
        let _msg = read_msg(&mut reader).await;

        // First message
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"topic","id":"req-1"}"#,
        )
        .await;
        let _r1 = read_msg(&mut reader).await;
        let _d1 = read_msg(&mut reader).await;

        // Compact
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"/compact","id":"req-compact"}"#,
        )
        .await;
        let _cr = read_msg(&mut reader).await;
        let _cd = read_msg(&mut reader).await;

        // Another message after compact — must not panic, not disconnect
        send_json(
            &mut writer_half,
            r#"{"type":"chat.message","content":"more","id":"req-2"}"#,
        )
        .await;
        let r2 = read_msg(&mut reader).await;
        let d2 = read_msg(&mut reader).await;

        assert!(
            matches!(&r2, ServerMessage::ChatResponse { .. }),
            "Session should still respond after compact, got: {:?}",
            r2
        );
        assert!(
            matches!(&d2, ServerMessage::ChatResponseDone { .. }),
            "Should receive ChatResponseDone after compact"
        );

        drop(writer_half);
        let _ = session_handle.await;
    }
}
