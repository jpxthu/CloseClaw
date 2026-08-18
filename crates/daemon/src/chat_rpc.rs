//! Chat RPC server — listens on a Unix domain socket and dispatches
//! chat messages through the Gateway's full inbound/outbound pipeline.
//!
//! Uses length-prefixed JSON frames (same protocol as admin RPC):
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```
//!
//! The server registers an `RpcTerminalPlugin` with the Gateway. When the
//! Gateway processes a message and produces output (via streaming or batch),
//! it calls `plugin.send()` with the rendered output, which is forwarded
//! over the RPC channel to the connected CLI client.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{
    AdapterError, IMPlugin, MessageType, NormalizedMessage, RenderedOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::DefaultStreamingRenderer;
use closeclaw_gateway::types::InboundChainInput;
use closeclaw_gateway::{Gateway, HandleResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};

use closeclaw_cli::chat::rpc::protocol::{ChatRequest, ChatResponse};
use closeclaw_cli::renderer::TerminalRenderer;

// Task-local connection ID — set by `dispatch_chat_message` before
// calling the Gateway so that `RpcTerminalPlugin::send()` can route
// to the correct per-connection channel.
tokio::task_local! {
    static CHAT_CONN_ID: u64;
}

// ---------------------------------------------------------------------------
// ChatContext
// ---------------------------------------------------------------------------

/// Server-side context holding a reference to the Gateway and the
/// shared RpcTerminalPlugin for per-connection channel routing.
pub struct ChatContext {
    pub gateway: Arc<Gateway>,
    /// The RpcTerminalPlugin registered with the Gateway.
    /// Stored here so `dispatch_chat_message` can access it without
    /// downcasting `Arc<dyn IMPlugin>`.
    pub rpc_plugin: Arc<RpcTerminalPlugin>,
}

// ---------------------------------------------------------------------------
// ChatRpcServer
// ---------------------------------------------------------------------------

/// Chat RPC server that binds a Unix domain socket and handles
/// incoming chat requests.
pub struct ChatRpcServer {
    path: PathBuf,
    context: Arc<ChatContext>,
}

impl ChatRpcServer {
    /// Create a new chat RPC server with the given socket path and context.
    pub fn new(path: impl Into<PathBuf>, context: ChatContext) -> Self {
        Self {
            path: path.into(),
            context: Arc::new(context),
        }
    }

    /// Remove the socket file if it already exists (idempotent).
    async fn clean_up(&self) {
        let _ = tokio::fs::remove_file(&self.path).await;
    }

    /// Start the chat RPC server. Blocks forever, processing each
    /// connection in a spawned task.
    pub async fn serve(self) -> std::io::Result<()> {
        self.clean_up().await;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("chat RPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let context = Arc::clone(&self.context);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, context).await {
                            tracing::error!("chat RPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("chat RPC accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single chat RPC connection.
async fn handle_connection(stream: UnixStream, context: Arc<ChatContext>) -> std::io::Result<()> {
    let (reader, mut writer): (_, OwnedWriteHalf) = stream.into_split();
    let mut reader = BufReader::new(reader);

    loop {
        // Read 4-byte length header
        let mut hdr = [0u8; 4];
        match reader.read_exact(&mut hdr).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        // Deserialize request
        let request: ChatRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = ChatResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                send_response(&mut writer, &resp).await?;
                continue;
            }
        };

        // Dispatch request — returns responses to send back
        let responses = dispatch(request, &context).await;
        for resp in &responses {
            send_response(&mut writer, resp).await?;
        }
    }

    Ok(())
}

/// Dispatch a chat request and return the responses to send back.
async fn dispatch(request: ChatRequest, context: &ChatContext) -> Vec<ChatResponse> {
    match request {
        ChatRequest::ChatMessage { agent_id, content } => {
            dispatch_chat_message(agent_id, content, context).await
        }
        ChatRequest::StopSession { agent_id } => dispatch_stop_session(agent_id, context).await,
        ChatRequest::Quit => vec![],
        ChatRequest::Ping => vec![ChatResponse::Pong],
    }
}

/// Drain remaining messages from the channel into the response vec.
fn drain_channel(rx: &mut mpsc::Receiver<RenderedOutput>, out: &mut Vec<ChatResponse>) {
    while let Ok(output) = rx.try_recv() {
        out.push(rendered_to_response(&output));
    }
}

/// Collect responses from the channel and gateway handle until done.
async fn collect_responses(
    mut rx: mpsc::Receiver<RenderedOutput>,
    mut handle: tokio::task::JoinHandle<Option<HandleResult>>,
) -> (Vec<ChatResponse>, mpsc::Receiver<RenderedOutput>) {
    let mut responses = Vec::new();
    loop {
        tokio::select! {
            rendered = rx.recv() => {
                match rendered {
                    Some(output) => {
                        responses.push(rendered_to_response(&output));
                    }
                    None => break,
                }
            }
            result = &mut handle => {
                match result {
                    Ok(Some(_)) => {}
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "chat message handler panicked");
                        responses.push(ChatResponse::Error {
                            message: format!("internal error: {}", e),
                        });
                    }
                }
                break;
            }
        }
    }

    // Unified drain after the select! loop.
    drain_channel(&mut rx, &mut responses);

    (responses, rx)
}

/// Finalize the response list: append Done or Error as appropriate.
fn finalize_responses(mut responses: Vec<ChatResponse>) -> Vec<ChatResponse> {
    if responses.is_empty() {
        responses.push(ChatResponse::Error {
            message: "no response from gateway".to_string(),
        });
    } else if !responses.iter().any(|r| matches!(r, ChatResponse::Done)) {
        responses.push(ChatResponse::Done);
    }
    responses
}

/// Set up an RPC channel and register it with the plugin.
///
/// Returns the channel receiver and a unique connection ID.
async fn setup_rpc_channel(context: &ChatContext) -> (mpsc::Receiver<RenderedOutput>, u64) {
    let (tx, rx) = mpsc::channel::<RenderedOutput>(64);

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let conn_id = COUNTER.fetch_add(1, Ordering::Relaxed);

    context.rpc_plugin.register_sender(conn_id, tx).await;
    (rx, conn_id)
}

/// Build an [`InboundChainInput`] from the chat message content.
fn build_inbound_input(content: String) -> InboundChainInput {
    let now_ms = chrono::Utc::now().timestamp_millis();
    InboundChainInput {
        platform: "terminal".to_string(),
        sender_id: closeclaw_platform::current_uid(),
        peer_id: "cli".to_string(),
        content,
        message_id: format!("chat-{}", now_ms),
        timestamp_ms: now_ms,
        account_id: Some("owner".to_string()),
        thread_id: None,
        message_type: MessageType::Text,
        media_refs: vec![],
        chat_name: None,
        trace_id: Some(format!("chat-{}", now_ms)),
    }
}

/// Process gateway responses and finalize the response list.
async fn process_gateway_response(
    rx: mpsc::Receiver<RenderedOutput>,
    conn_id: u64,
    agent_id: String,
    content: String,
    context: &ChatContext,
) -> Vec<ChatResponse> {
    let input = build_inbound_input(content);

    // Run the inbound processor chain
    // (RawLog → SessionRouter → ContentNormalizer).
    let processed = context.gateway.process_inbound_chain(&input).await;

    // Dispatch through Gateway: resolves session, routes to LLM or slash
    // command.
    let gw = Arc::clone(&context.gateway);
    let handle = tokio::spawn(CHAT_CONN_ID.scope(conn_id, async move {
        gw.handle_inbound_message(processed, Some(&agent_id), "terminal")
            .await
    }));

    let (responses, _rx) = collect_responses(rx, handle).await;

    // Unregister the channel sender.
    context.rpc_plugin.unregister_sender(conn_id).await;

    finalize_responses(responses)
}

/// Handle a chat message: route through Gateway's full inbound/outbound
/// pipeline.
///
/// Registers a per-request channel on the shared RpcTerminalPlugin,
/// calls the Gateway, and reads responses from the channel until done.
async fn dispatch_chat_message(
    agent_id: String,
    content: String,
    context: &ChatContext,
) -> Vec<ChatResponse> {
    let (rx, conn_id) = setup_rpc_channel(context).await;
    process_gateway_response(rx, conn_id, agent_id, content, context).await
}

/// Convert a [`RenderedOutput`] to a [`ChatResponse`].
fn rendered_to_response(output: &RenderedOutput) -> ChatResponse {
    match output.msg_type.as_str() {
        "text" => {
            let text = extract_text_from_payload(&output.payload);
            ChatResponse::ContentChunk { text }
        }
        "interactive" => {
            let text = serde_json::to_string(&output.payload)
                .unwrap_or_else(|_| output.payload.to_string());
            ChatResponse::ContentChunk { text }
        }
        other => {
            let text = extract_text_from_payload(&output.payload);
            if text.is_empty() {
                tracing::warn!(msg_type = other, "unknown RenderedOutput type");
                ChatResponse::ContentChunk {
                    text: output.payload.to_string(),
                }
            } else {
                ChatResponse::ContentChunk { text }
            }
        }
    }
}

/// Extract text content from a RenderedOutput payload.
fn extract_text_from_payload(payload: &serde_json::Value) -> String {
    if let Some(text) = payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
    {
        return text.to_string();
    }
    if let Some(text) = payload.as_str() {
        return text.to_string();
    }
    payload.to_string()
}

/// Handle a stop session request.
async fn dispatch_stop_session(agent_id: String, context: &ChatContext) -> Vec<ChatResponse> {
    let sender_id = closeclaw_platform::current_uid();
    let now_ms = chrono::Utc::now().timestamp_millis();

    let input = InboundChainInput {
        platform: "terminal".to_string(),
        sender_id,
        peer_id: "cli".to_string(),
        content: "/stop".to_string(),
        message_id: format!("stop-{}", now_ms),
        timestamp_ms: now_ms,
        account_id: Some("owner".to_string()),
        thread_id: None,
        message_type: MessageType::Text,
        media_refs: vec![],
        chat_name: None,
        trace_id: None,
    };

    let processed = context.gateway.process_inbound_chain(&input).await;

    match context
        .gateway
        .handle_inbound_message(processed, Some(&agent_id), "terminal")
        .await
    {
        Some(_) => vec![ChatResponse::Done],
        None => vec![ChatResponse::Error {
            message: format!("failed to stop session for agent '{}'", agent_id),
        }],
    }
}

/// Send a length-prefixed JSON response.
async fn send_response(
    writer: &mut OwnedWriteHalf,
    response: &ChatResponse,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// RpcTerminalPlugin
// ---------------------------------------------------------------------------

/// Terminal output plugin for RPC-based chat connections.
///
/// Implements [`IMPlugin`] to receive rendered output from the Gateway
/// and forward it through per-connection [`mpsc`] channels to the Chat
/// RPC connection handlers.
///
/// Registered once at daemon startup. Each incoming chat request registers
/// a sender keyed by a monotonically increasing connection ID, so concurrent
/// connections are safely routed without modifying the global plugin table.
pub struct RpcTerminalPlugin {
    /// Per-connection senders: conn_id → mpsc::Sender.
    connections: RwLock<HashMap<u64, mpsc::Sender<RenderedOutput>>>,
    /// Streaming renderer for handling incremental LLM output.
    streaming_renderer: std::sync::Mutex<DefaultStreamingRenderer>,
    /// Terminal renderer for ANSI-aware content block rendering.
    renderer: TerminalRenderer,
}

impl RpcTerminalPlugin {
    /// Create a new RPC terminal plugin.
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            streaming_renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
            renderer: TerminalRenderer::new(),
        }
    }

    /// Register a sender for the given connection ID.
    pub async fn register_sender(&self, conn_id: u64, sender: mpsc::Sender<RenderedOutput>) {
        let mut conns = self.connections.write().await;
        conns.insert(conn_id, sender);
    }

    /// Unregister the sender for the given connection ID.
    pub async fn unregister_sender(&self, conn_id: u64) {
        let mut conns = self.connections.write().await;
        conns.remove(&conn_id);
    }
}

impl Default for RpcTerminalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IMPlugin for RpcTerminalPlugin {
    fn platform(&self) -> &str {
        "terminal"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        // RPC connections don't parse inbound payloads — messages arrive
        // as ChatRequest::ChatMessage.
        Ok(None)
    }

    fn streaming_renderer(&self) -> Option<&std::sync::Mutex<DefaultStreamingRenderer>> {
        Some(&self.streaming_renderer)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        // Delegate to TerminalRenderer for proper ANSI-aware rendering
        // of Thinking, ToolUse, ToolResult, and DSL blocks.
        self.renderer.render(content_blocks, dsl_result)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        // Route to the current task's connection channel.
        let conn_id = CHAT_CONN_ID.with(|id| *id);
        let conns = self.connections.read().await;
        let sender = conns
            .get(&conn_id)
            .ok_or_else(|| AdapterError::SendFailed(format!("connection {} not found", conn_id)))?;
        sender
            .send(output.clone())
            .await
            .map_err(|e| AdapterError::SendFailed(e.to_string()))
    }

    fn clean_content(&self, raw: &str) -> String {
        raw.to_string()
    }

    async fn init(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), AdapterError> {
        let mut conns = self.connections.write().await;
        conns.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Socket path helper
// ---------------------------------------------------------------------------

/// Return the chat RPC socket path for the given config directory.
pub fn chat_socket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat.sock")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
            .scope(conn_id, plugin.send(&output, "peer", None))
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
            .scope(conn1, plugin.send(&out1, "peer", None))
            .await
            .unwrap();

        // Send on connection 2 using task-local scope.
        let out2 = RenderedOutput {
            msg_type: "text".to_string(),
            payload: json!("msg2"),
        };
        CHAT_CONN_ID
            .scope(conn2, plugin.send(&out2, "peer", None))
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
            .scope(1, plugin.send(&output, "peer", None))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[should_panic(expected = "cannot access a task-local storage value without setting it first")]
    async fn test_rpc_terminal_plugin_send_no_task_local() {
        let plugin = RpcTerminalPlugin::new();
        let (tx, _rx) = mpsc::channel(4);
        plugin.register_sender(1, tx).await;
        // Calling send() without CHAT_CONN_ID scope should panic.
        let output = RenderedOutput {
            msg_type: "text".to_string(),
            payload: json!("test"),
        };
        let _ = plugin.send(&output, "peer", None).await;
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
                    .scope(i as u64, plugin.send(&out, "peer", None))
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
}
