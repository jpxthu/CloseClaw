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
            text: "hello".to_string()
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
        ChatResponse::ContentChunk { text } => {
            assert!(text.contains("card"));
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

#[tokio::test]
async fn test_rpc_terminal_plugin_shutdown_clears_connections() {
    let plugin = RpcTerminalPlugin::new();
    let (tx, _rx) = mpsc::channel(4);
    plugin.register_sender(1, tx).await;
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
            text: "fallback".to_string()
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

/// Concurrent RPC connections must not interfere with each other.
#[tokio::test]
async fn test_concurrent_rpc_connections_no_race() {
    let plugin = Arc::new(RpcTerminalPlugin::new());
    let num_connections = 5;

    // Set up channels for each connection.
    let mut receivers: Vec<mpsc::Receiver<RenderedOutput>> = Vec::new();
    for i in 0..num_connections {
        let (tx, rx) = mpsc::channel(4);
        plugin.register_sender(i as u64, tx).await;
        receivers.push(rx);
    }

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
                .scope(i as u64, plugin.send(&out, "peer", None, None))
                .await
        });
        handles.push((i, handle));
    }

    // Wait for all sends to complete.
    for (_i, handle) in handles {
        handle.await.unwrap().unwrap();
    }

    // Verify each connection received exactly its own message.
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

    // Clean up.
    for i in 0..num_connections {
        plugin.unregister_sender(i as u64).await;
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
