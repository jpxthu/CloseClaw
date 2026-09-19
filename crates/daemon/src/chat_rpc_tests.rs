use super::*;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_gateway::SessionManager;
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_extract_text_from_content_payload() {
    let payload = json!({"content": {"text": "hello world"}});
    assert_eq!(extract_text_from_payload(&payload), "hello world");
}

#[test]
fn test_extract_text_from_raw_string() {
    let payload = json!("plain text");
    assert_eq!(extract_text_from_payload(&payload), "plain text");
}

#[test]
fn test_extract_text_from_object_fallback() {
    let payload = json!({"key": "value"});
    let result = extract_text_from_payload(&payload);
    assert!(result.contains("key"));
    assert!(result.contains("value"));
}

#[test]
fn test_rendered_to_response_text() {
    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!({"content": {"text": "hello"}}),
    };
    let resp = rendered_to_response(&output);
    assert_eq!(
        resp,
        ChatResponse::ContentChunk {
            content: "hello".to_string()
        }
    );
}

#[test]
fn test_rendered_to_response_interactive() {
    let output = RenderedOutput {
        msg_type: "interactive".to_string(),
        payload: json!({"card": {"header": {"title": "test"}}}),
    };
    let resp = rendered_to_response(&output);
    match resp {
        ChatResponse::ContentChunk { content } => {
            assert!(content.contains("card"));
        }
        _ => panic!("expected ContentChunk"),
    }
}

#[test]
fn test_rpc_terminal_plugin_render() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![ContentBlock::Text("line1".to_string())];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
    // TerminalRenderer adds trailing newlines from markdown rendering
    // and an additional newline per block.
    assert_eq!(output.payload, json!("line1\n\n"));
}

#[test]
fn test_rpc_terminal_plugin_render_multiple_blocks() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![
        ContentBlock::Text("line1".to_string()),
        ContentBlock::Text("line2".to_string()),
    ];
    let output = plugin.render(&blocks, None);
    // TerminalRenderer adds a newline after each block.
    assert_eq!(output.payload, json!("line1\n\nline2\n\n"));
}

#[test]
fn test_rpc_terminal_plugin_platform() {
    let plugin = RpcTerminalPlugin::new();
    assert_eq!(plugin.platform(), "terminal");
}

#[tokio::test]
async fn test_rpc_terminal_plugin_send_via_channel() {
    let plugin = RpcTerminalPlugin::new();
    let (tx, mut rx) = mpsc::channel(4);

    let conn_id = 42u64;
    plugin.register_sender(conn_id, tx).await;

    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("test message"),
    };

    // Simulate the task-local scope that dispatch_chat_message sets.
    let result = CHAT_CONN_ID
        .scope(conn_id, plugin.send(&output, "peer", None, None))
        .await;
    result.unwrap();
    let received = rx.recv().await.unwrap();
    assert_eq!(received, output);

    plugin.unregister_sender(conn_id).await;
}

#[tokio::test]
async fn test_rpc_terminal_plugin_concurrent_connections() {
    let plugin = RpcTerminalPlugin::new();
    let (tx1, mut rx1) = mpsc::channel(4);
    let (tx2, mut rx2) = mpsc::channel(4);

    let conn1 = 1u64;
    let conn2 = 2u64;
    plugin.register_sender(conn1, tx1).await;
    plugin.register_sender(conn2, tx2).await;

    // Send on connection 1 using task-local scope.
    let out1 = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("msg1"),
    };
    CHAT_CONN_ID
        .scope(conn1, plugin.send(&out1, "peer", None, None))
        .await
        .unwrap();

    // Send on connection 2 using task-local scope.
    let out2 = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("msg2"),
    };
    CHAT_CONN_ID
        .scope(conn2, plugin.send(&out2, "peer", None, None))
        .await
        .unwrap();

    // Verify each channel got its own message.
    let r1 = rx1.recv().await.unwrap();
    assert_eq!(r1.payload, json!("msg1"));
    let r2 = rx2.recv().await.unwrap();
    assert_eq!(r2.payload, json!("msg2"));

    plugin.unregister_sender(conn1).await;
    plugin.unregister_sender(conn2).await;
}

/// Shutdown must clear agent routes together with the senders (pair
/// cleanup via `clear_connections`, Step 1.23): a stale route after
/// shutdown would resolve a task-local-less send and fail with the
/// misleading "connection not found" branch instead of "no route".
#[tokio::test]
async fn test_rpc_terminal_plugin_shutdown_clears_connections() {
    let plugin = RpcTerminalPlugin::new();
    let (tx, _rx) = mpsc::channel(4);
    plugin.register_sender(1, tx).await;
    plugin.register_agent_route("master", 1).await;
    plugin.shutdown().await.unwrap();

    // After shutdown, send should fail.
    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("test"),
    };
    let result = CHAT_CONN_ID
        .scope(1, plugin.send(&output, "peer", None, None))
        .await;
    assert!(result.is_err());

    // Route map cleared with the senders — acceptance for Step 1.23.
    assert!(
        plugin.agent_routes.read().await.is_empty(),
        "agent routes must be cleared on shutdown"
    );
    let err = plugin
        .send(&output, "master", None, None)
        .await
        .expect_err("post-shutdown route-less send must fail");
    assert!(
        err.to_string().contains("no chat connection registered"),
        "must miss the route, not hit a dead connection id: {err}"
    );
}

#[tokio::test]
async fn test_rpc_terminal_plugin_send_no_task_local_no_route() {
    let plugin = RpcTerminalPlugin::new();
    let (tx, _rx) = mpsc::channel(4);
    plugin.register_sender(1, tx).await;
    // Without CHAT_CONN_ID scope and no registered agent route, send()
    // fails gracefully instead of panicking.
    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("test"),
    };
    let err = plugin
        .send(&output, "peer", None, None)
        .await
        .expect_err("send without task-local or agent route should fail");
    assert!(matches!(err, AdapterError::SendFailed(_)));
}

#[tokio::test]
async fn test_rpc_terminal_plugin_send_via_agent_route() {
    let plugin = RpcTerminalPlugin::new();
    let (tx, mut rx) = mpsc::channel(4);
    plugin.register_sender(7, tx).await;
    plugin.register_agent_route("peer", 7).await;
    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("test"),
    };
    // No task-local (LLM dispatch task): routes via the agent fallback.
    plugin
        .send(&output, "peer", None, None)
        .await
        .expect("send via agent route should succeed");
    assert!(rx.try_recv().is_ok());
}

#[test]
fn test_chat_socket_path() {
    let path = chat_socket_path(Path::new("/home/user/.closeclaw"));
    assert_eq!(path, PathBuf::from("/home/user/.closeclaw/chat.sock"));
}

#[test]
fn test_extract_text_empty_payload() {
    let payload = json!({});
    let result = extract_text_from_payload(&payload);
    assert_eq!(result, "{}");
}

#[test]
fn test_rendered_to_response_unknown_type() {
    let output = RenderedOutput {
        msg_type: "unknown_type".to_string(),
        payload: json!({"content": {"text": "fallback"}}),
    };
    let resp = rendered_to_response(&output);
    assert_eq!(
        resp,
        ChatResponse::ContentChunk {
            content: "fallback".to_string()
        }
    );
}

#[test]
fn test_drain_channel_empty() {
    let (tx, mut rx) = mpsc::channel(4);
    drop(tx);
    let mut out = Vec::new();
    drain_channel(&mut rx, &mut out);
    assert!(out.is_empty());
}

#[test]
fn test_drain_channel_with_messages() {
    let (tx, mut rx) = mpsc::channel(4);
    let out1 = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("a"),
    };
    let out2 = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("b"),
    };
    tx.try_send(out1).unwrap();
    tx.try_send(out2).unwrap();
    drop(tx);

    let mut out = Vec::new();
    drain_channel(&mut rx, &mut out);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_dispatch_ping_returns_pong() {
    // Ping is handled synchronously in dispatch(), verify the variant.
    let req = ChatRequest::Ping;
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("ping"));
}

// ── Step 1.10: supplementary tests ──────────────────────────────────────

/// sender_id must be the system UID, not agent_id.
#[test]
fn test_build_inbound_input_sender_id_is_system_uid() {
    let input = build_inbound_input("test content".to_string());
    let expected_uid = closeclaw_platform::current_uid();
    assert_eq!(
        input.sender_id, expected_uid,
        "sender_id should be system UID, not agent_id"
    );
}

/// The request's agent_id must be attached to processed metadata so
/// Gateway session resolution routes by it (cli/chat.md --agent-id).
#[test]
fn test_attach_target_agent_sets_metadata() {
    let mut processed =
        closeclaw_common::processor::ProcessedMessage::from_raw_content("hi".to_string());
    attach_target_agent(&mut processed, "master");
    assert_eq!(processed.metadata.get("agent_id").unwrap(), "master");
}

/// RpcTerminalPlugin::render() must correctly render Thinking blocks.
#[test]
fn test_rpc_terminal_plugin_render_thinking() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![ContentBlock::Thinking {
        thinking: "reasoning here".to_string(),
        signature: None,
    }];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
    let text = output.payload.as_str().unwrap_or("");
    assert!(
        text.contains("[Thinking]"),
        "rendered output should contain [Thinking] marker"
    );
    assert!(
        text.contains("reasoning here"),
        "rendered output should contain thinking content"
    );
    assert!(
        text.contains("[end of thinking]"),
        "rendered output should contain [end of thinking] marker"
    );
}

/// RpcTerminalPlugin::render() must correctly render ToolUse blocks.
#[test]
fn test_rpc_terminal_plugin_render_tool_use() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![ContentBlock::ToolUse {
        name: "web_search".to_string(),
        input: r#"{"query":"rust async"}"#.to_string(),
        id: "tool-1".to_string(),
    }];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
    let text = output.payload.as_str().unwrap_or("");
    assert!(
        text.contains("web_search"),
        "rendered output should contain tool name"
    );
    assert!(
        text.contains("rust async"),
        "rendered output should contain tool input"
    );
}

/// RpcTerminalPlugin::render() must correctly render ToolResult blocks.
#[test]
fn test_rpc_terminal_plugin_render_tool_result() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tool-1".to_string(),
        content: "found 3 results".to_string(),
    }];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
    let text = output.payload.as_str().unwrap_or("");
    assert!(
        text.contains("found 3 results"),
        "rendered output should contain tool result content"
    );
}

/// RpcTerminalPlugin::render() handles mixed block types in one call.
#[test]
fn test_rpc_terminal_plugin_render_mixed_blocks() {
    let plugin = RpcTerminalPlugin::new();
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "step 1".to_string(),
            signature: None,
        },
        ContentBlock::ToolUse {
            name: "read".to_string(),
            input: "{}".to_string(),
            id: "t1".to_string(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "t1".to_string(),
            content: "file contents".to_string(),
        },
        ContentBlock::Text("final answer".to_string()),
    ];
    let output = plugin.render(&blocks, None);
    let text = output.payload.as_str().unwrap_or("");
    assert!(text.contains("[Thinking]"));
    assert!(text.contains("step 1"));
    assert!(text.contains("read"));
    assert!(text.contains("file contents"));
    assert!(text.contains("final answer"));
}

/// Register one sender per connection on `plugin`, returning the
/// receivers in connection order (ids `0..num_connections`).
async fn register_connections(
    plugin: &Arc<RpcTerminalPlugin>,
    num_connections: u64,
) -> Vec<mpsc::Receiver<RenderedOutput>> {
    let mut receivers: Vec<mpsc::Receiver<RenderedOutput>> = Vec::new();
    for i in 0..num_connections {
        let (tx, rx) = mpsc::channel(4);
        plugin.register_sender(i, tx).await;
        receivers.push(rx);
    }
    receivers
}

/// Assert each connection received exactly its own message — no
/// cross-talk between connections, no extra messages leaked in.
async fn assert_each_connection_received_own_message(
    receivers: Vec<mpsc::Receiver<RenderedOutput>>,
) {
    for (i, mut rx) in receivers.into_iter().enumerate() {
        let output = rx.recv().await.unwrap();
        assert_eq!(
            output.payload,
            json!(format!("msg-{}", i)),
            "connection {} should receive its own message",
            i
        );
        // Ensure no extra messages leaked from other connections.
        assert!(
            rx.try_recv().is_err(),
            "connection {} should not have extra messages",
            i
        );
    }
}

/// Concurrent RPC connections must not interfere with each other.
#[tokio::test]
async fn test_concurrent_rpc_connections_no_race() {
    let plugin = Arc::new(RpcTerminalPlugin::new());
    let num_connections = 5u64;

    // Set up channels for each connection.
    let receivers = register_connections(&plugin, num_connections).await;

    // Simulate concurrent sends on each connection.
    let mut handles = Vec::new();
    for i in 0..num_connections {
        let plugin = Arc::clone(&plugin);
        let handle = tokio::spawn(async move {
            let out = RenderedOutput {
                msg_type: "text".to_string(),
                payload: json!(format!("msg-{}", i)),
            };
            CHAT_CONN_ID
                .scope(i, plugin.send(&out, "peer", None, None))
                .await
        });
        handles.push((i, handle));
    }

    // Wait for all sends to complete.
    for (_i, handle) in handles {
        handle.await.unwrap().unwrap();
    }

    // Verify each connection received exactly its own message.
    assert_each_connection_received_own_message(receivers).await;

    // Clean up.
    for i in 0..num_connections {
        plugin.unregister_sender(i).await;
    }
}

/// dispatch() with ChatRequest::Ping must return ChatResponse::Pong
/// without side effects.
#[tokio::test]
async fn test_dispatch_ping_returns_pong_actual() {
    let req = ChatRequest::Ping;
    let context = ChatContext {
        gateway: Arc::new(closeclaw_gateway::Gateway::new(
            closeclaw_gateway::types::GatewayConfig::default(),
            Arc::new(SessionManager::new(
                &closeclaw_gateway::types::GatewayConfig::default(),
                None,
                None,
                closeclaw_common::ReasoningLevel::default(),
            )),
        )),
        rpc_plugin: Arc::new(RpcTerminalPlugin::new()),
    };
    let responses = dispatch(req, &context).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0], ChatResponse::Pong);
}

/// Step 1.15 — failure-side turn completion: the shared consumer must turn
/// the EMPTY payload emitted by the gateway's failed LLM turn
/// (`session_handler_announce` error arm sends `("", [])`) into a
/// `finish_turns()` call, so waiting chat connections finalize instead of
/// hanging until `TURN_COMPLETION_TIMEOUT_SECS` (120s). The emission half
/// is locked by the gateway's
/// `session_handler_announce_turn_completion_tests`.
#[tokio::test]
async fn test_turn_completion_consumer_finalizes_failed_empty_payload_turn() {
    // Waiting chat connection + shared consumer (Step 1.20 harness).
    let mut h = crate::test_helpers::setup_turn_completion_consumer().await;

    // Exactly what the failure arm emits for a failed turn: empty payload.
    h.output_tx
        .send((String::new(), Vec::new()))
        .await
        .expect("output channel must be open");
    let closed = tokio::time::timeout(std::time::Duration::from_secs(1), h.conn_rx.recv())
        .await
        .expect("failed turn must finalize, not hang until the 120s timeout");
    assert!(
        closed.is_none(),
        "connection channel must close on finish_turns for the empty failure payload"
    );

    // Dropping the output sender closes the consumer loop.
    drop(h.output_tx);
    h.consumer
        .await
        .expect("consumer task must exit when the output channel closes");
}

/// Step 1.20 — `unregister_sender` must also drop agent routes pointing
/// at that connection: a stale route would resolve task-local-less sends
/// to a dead connection id ("connection not found") or keep routing to
/// an agent whose chat connection is gone.
#[tokio::test]
async fn test_unregister_sender_clears_agent_route() {
    let plugin = Arc::new(RpcTerminalPlugin::new());
    let (tx, _rx) = mpsc::channel(4);
    plugin.register_sender(1, tx).await;
    plugin.register_agent_route("master", 1).await;
    assert!(
        plugin.agent_routes.read().await.contains_key("master"),
        "route registered"
    );

    plugin.unregister_sender(1).await;
    assert!(
        plugin.agent_routes.read().await.is_empty(),
        "route must be cleared together with its connection"
    );

    // Functional consequence: a task-local-less send now finds no route.
    let out = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("hi"),
    };
    let err = plugin.send(&out, "master", None, None).await.unwrap_err();
    assert!(
        err.to_string().contains("no chat connection registered"),
        "{err}"
    );
}

/// Step 1.20 — latest registration wins for the same agent (the rule
/// documented on `agent_routes`): a later connection supersedes the
/// earlier route, and task-local-less sends land on the latest one only.
#[tokio::test]
async fn test_agent_route_latest_registration_wins() {
    let plugin = Arc::new(RpcTerminalPlugin::new());
    let (tx1, mut rx1) = mpsc::channel(4);
    let (tx2, mut rx2) = mpsc::channel(4);
    plugin.register_sender(1, tx1).await;
    plugin.register_sender(2, tx2).await;
    plugin.register_agent_route("master", 1).await;
    plugin.register_agent_route("master", 2).await;
    assert_eq!(
        plugin.agent_routes.read().await.get("master"),
        Some(&2),
        "the latest registration must win"
    );

    let out = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("hi"),
    };
    plugin
        .send(&out, "master", None, None)
        .await
        .expect("route registered");
    assert!(
        rx2.recv().await.is_some(),
        "the latest registration must receive"
    );
    assert!(
        rx1.try_recv().is_err(),
        "the superseded connection must not receive"
    );
}

/// Step 1.21 — non-LlmStarted handler results take the drain branch:
/// synchronous output already queued on the connection channel is
/// collected and the function returns without waiting for the channel
/// to close (the guard deadline fails the test if the wait loop is
/// entered instead of draining).
#[tokio::test]
async fn test_collect_responses_non_llm_drains_queued_output() {
    let (tx, rx) = mpsc::channel(4);
    let output = RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("sync reply"),
    };
    tx.send(output).await.unwrap();
    // Non-LlmStarted result: synchronous handling finished in the task.
    let handle: tokio::task::JoinHandle<Option<HandleResult>> =
        tokio::spawn(async { Some(HandleResult::SlashHandled) });

    let (responses, mut rx_left) = tokio::time::timeout(
        Duration::from_millis(500),
        collect_responses_with_timeout(rx, handle, Duration::from_secs(30)),
    )
    .await
    .expect("drain branch must return without waiting for channel close");

    assert_eq!(
        responses,
        vec![ChatResponse::ContentChunk {
            content: "sync reply".to_string()
        }],
        "queued synchronous output must be drained into the responses"
    );
    assert!(
        matches!(rx_left.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "connection channel must stay open (sender alive) with no queued leftovers"
    );
    drop(tx);
}

/// Step 1.23 — normal close path: an LlmStarted handler whose
/// connection channel closes (turn-completion consumer ran
/// `finish_turns`, or the sender was dropped) returns promptly with the
/// queued frames instead of waiting out the injected bound.
#[tokio::test]
async fn test_collect_responses_llm_started_returns_on_channel_close() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(RenderedOutput {
        msg_type: "text".to_string(),
        payload: json!("final frame"),
    })
    .await
    .unwrap();
    // Close the channel — the LlmStarted wait loop must observe the
    // close and finalize immediately.
    drop(tx);
    let handle: tokio::task::JoinHandle<Option<HandleResult>> =
        tokio::spawn(async { Some(HandleResult::LlmStarted) });

    let (responses, mut rx_left) = tokio::time::timeout(
        Duration::from_millis(500),
        collect_responses_with_timeout(rx, handle, Duration::from_secs(30)),
    )
    .await
    .expect("a closed channel must finalize without waiting for the bound");

    assert_eq!(
        responses,
        vec![ChatResponse::ContentChunk {
            content: "final frame".to_string()
        }],
        "queued output must be collected before the close"
    );
    assert!(
        matches!(
            rx_left.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ),
        "the connection channel must be observed closed"
    );
}

/// Step 1.23 — panic path: a handler task that panics surfaces as a
/// single `Error` frame ("internal error: …") instead of losing the
/// turn; the connection channel itself is left untouched.
#[tokio::test]
async fn test_collect_responses_handler_panic_yields_error_frame() {
    let (_tx, rx) = mpsc::channel(4); // sender alive — only the task fails
    let handle: tokio::task::JoinHandle<Option<HandleResult>> =
        tokio::spawn(async { panic!("handler exploded") });

    let (responses, mut rx_left) = tokio::time::timeout(
        Duration::from_millis(500),
        collect_responses_with_timeout(rx, handle, Duration::from_secs(30)),
    )
    .await
    .expect("a panicking handler must be surfaced promptly");

    assert_eq!(
        responses.len(),
        1,
        "exactly one terminal frame: {responses:?}"
    );
    match &responses[0] {
        ChatResponse::Error { message } => {
            assert!(message.contains("internal error"), "got: {message}");
        }
        other => panic!("expected an Error frame, got {other:?}"),
    }
    assert!(
        matches!(rx_left.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "the connection channel must stay open and untouched"
    );
}

/// Step 1.21 — the LlmStarted wait honors the injected bound: a turn
/// that never signals completion returns at the bound instead of
/// hanging (production keeps the `TURN_COMPLETION_TIMEOUT_SECS` = 120s
/// default via `collect_responses`), and the channel is left untouched.
#[tokio::test]
async fn test_collect_responses_llm_started_times_out_at_injected_bound() {
    let (_tx, rx) = mpsc::channel(4); // sender kept alive — channel never closes
    let handle: tokio::task::JoinHandle<Option<HandleResult>> =
        tokio::spawn(async { Some(HandleResult::LlmStarted) });

    let started = std::time::Instant::now();
    let (responses, mut rx_left) =
        collect_responses_with_timeout(rx, handle, Duration::from_millis(50)).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(50),
        "must wait up to the injected bound before giving up, elapsed {elapsed:?}"
    );
    assert!(
        responses.is_empty(),
        "no output arrived before the bound, got {responses:?}"
    );
    assert!(
        matches!(rx_left.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "the timeout must leave the connection channel open and untouched"
    );
}

/// Step 1.24 — concurrent drain while awaiting the handler: a handler
/// that synchronously fills the channel (capacity 2) blocks on its
/// third `send` until the collector receives, so the frames can only
/// be collected if `collect_responses_with_timeout` drains
/// concurrently with `handle.await`. All three frames must arrive in
/// order and the non-LlmStarted branch must return without hanging
/// (the guard deadline fails the test if nothing drains while the
/// handle is pending).
#[tokio::test]
async fn test_collect_responses_drains_while_awaiting_handler() {
    let (tx, rx) = mpsc::channel(2); // capacity < frames pushed: the 3rd send needs a recv
    let handle: tokio::task::JoinHandle<Option<HandleResult>> = tokio::spawn(async move {
        for i in 0..3u8 {
            tx.send(RenderedOutput {
                msg_type: "text".to_string(),
                payload: json!(format!("frame {i}")),
            })
            .await
            .expect("collector must drain while the handle is pending");
        }
        Some(HandleResult::SlashHandled)
    });

    let (responses, _rx_left) = tokio::time::timeout(
        Duration::from_secs(1),
        collect_responses_with_timeout(rx, handle, Duration::from_secs(30)),
    )
    .await
    .expect("must drain concurrently instead of deadlocking on a full channel");

    assert_eq!(
        responses,
        vec![
            ChatResponse::ContentChunk {
                content: "frame 0".to_string()
            },
            ChatResponse::ContentChunk {
                content: "frame 1".to_string()
            },
            ChatResponse::ContentChunk {
                content: "frame 2".to_string()
            },
        ],
        "every frame pushed by the handler must be collected, in order"
    );
}

// ── Step 1.4 (issue #3058): StopSession output-frame contract ──────────────

/// Indexes of non-empty content (reply) frames in a response sequence.
fn reply_frame_indexes(responses: &[ChatResponse]) -> Vec<usize> {
    responses
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            matches!(r, ChatResponse::ContentChunk { content } if !content.trim().is_empty())
        })
        .map(|(i, _)| i)
        .collect()
}

/// Indexes of terminal frames (`Done` / `Error`) in a response sequence.
fn terminal_frame_indexes(responses: &[ChatResponse]) -> Vec<usize> {
    responses
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r, ChatResponse::Done | ChatResponse::Error { .. }))
        .map(|(i, _)| i)
        .collect()
}

/// Build a [`ChatContext`] that can actually produce a `/stop` reply frame:
/// terminal IM plugin registered with the Gateway (outbound destination) and
/// a `StopHandler`-backed slash dispatcher installed (`/stop` →
/// `SlashResult::Stop` → "已停止当前任务" reply → outbound → plugin send →
/// agent-route fallback → this connection's channel).
async fn make_stop_ready_context() -> ChatContext {
    // max_message_size must be non-zero: 0 rejects every inbound message
    // in `validate_inbound` ("/stop" would exceed the 0-byte limit).
    let config = closeclaw_gateway::types::GatewayConfig {
        name: "stop-test".to_owned(),
        max_message_size: 64 * 1024,
        ..Default::default()
    };
    let sessions = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let gateway = Arc::new(Gateway::new(config, sessions));
    let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
    gateway
        .register_plugin(rpc_plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
        .await;
    let registry = Arc::new(closeclaw_slash::registry::HandlerRegistry::new());
    registry.register(Arc::new(closeclaw_slash::handlers_session::StopHandler));
    let dispatcher = Arc::new(closeclaw_slash::dispatcher::SlashDispatcher::from_shared(
        registry,
    )) as Arc<dyn closeclaw_common::SlashRouter>;
    gateway.set_slash_dispatcher(dispatcher).await;
    ChatContext {
        gateway,
        rpc_plugin,
    }
}

/// Step 1.4 — lock the `ChatRequest::StopSession` output-frame contract of
/// `dispatch_stop_session`: the agent route is registered, the `/stop` reply
/// frames produced by the stop chain are drained, and they all arrive
/// BEFORE the single terminal frame. A dropped route/drain loses the reply
/// (assertion ①), an inverted order fails ③, a duplicated terminal fails ② —
/// no regression can pass silently.
#[tokio::test]
async fn test_dispatch_stop_session_replies_before_terminal() {
    let context = make_stop_ready_context().await;
    let responses = dispatch(
        ChatRequest::StopSession {
            agent_id: "stop-agent".to_owned(),
        },
        &context,
    )
    .await;

    // ① at least one non-empty stop reply frame
    let replies = reply_frame_indexes(&responses);
    assert!(
        !replies.is_empty(),
        "stop reply frame must be present (route + drain), got {responses:?}"
    );

    // ② exactly one terminal frame, no more no less
    let terminals = terminal_frame_indexes(&responses);
    assert_eq!(
        terminals.len(),
        1,
        "exactly one terminal frame expected, got {responses:?}"
    );

    // ③ every reply frame precedes the terminal frame
    let last_reply = *replies.last().expect("reply list verified non-empty");
    assert!(
        last_reply < terminals[0],
        "reply frames must precede the terminal frame, got {responses:?}"
    );
}
