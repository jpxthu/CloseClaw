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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{
    AdapterError, IMPlugin, MessageType, NormalizedMessage, RenderedOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::DefaultStreamingRenderer;
use closeclaw_gateway::types::InboundChainInput;
use closeclaw_gateway::Gateway;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use closeclaw_cli::chat::rpc::protocol::{ChatRequest, ChatResponse};

// ---------------------------------------------------------------------------
// ChatContext
// ---------------------------------------------------------------------------

/// Server-side context holding a reference to the Gateway.
pub struct ChatContext {
    pub gateway: Arc<Gateway>,
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
///
/// For `ChatMessage`, the Gateway processes the message asynchronously.
/// The plugin's `send()` method forwards rendered output through a channel,
/// which we read and convert to `ChatResponse::ContentChunk`.
async fn dispatch(request: ChatRequest, context: &ChatContext) -> Vec<ChatResponse> {
    match request {
        ChatRequest::ChatMessage { agent_id, content } => {
            dispatch_chat_message(agent_id, content, context).await
        }
        ChatRequest::StopSession { agent_id } => dispatch_stop_session(agent_id, context).await,
        ChatRequest::Quit => vec![],
    }
}

/// Handle a chat message: route through Gateway's full inbound/outbound pipeline.
///
/// Creates a channel pair, sets the sender on the RpcTerminalPlugin, calls
/// the Gateway, and reads responses from the channel until done.
async fn dispatch_chat_message(
    agent_id: String,
    content: String,
    context: &ChatContext,
) -> Vec<ChatResponse> {
    // Create a bounded channel for response forwarding.
    // The producer (RpcTerminalPlugin::send) writes RenderedOutput;
    // the consumer (this function) reads and converts to ChatResponse.
    let (tx, mut rx) = mpsc::channel::<RenderedOutput>(64);

    // Create and configure the RPC terminal plugin for this request.
    let plugin: Arc<dyn IMPlugin> = Arc::new(RpcTerminalPlugin::new(tx));

    // Temporarily register the RpcTerminalPlugin with the Gateway so that
    // output from the inbound/outbound pipeline is routed through our channel.
    // Save the original plugin to restore after processing.
    let original_plugin = context.gateway.get_plugin("terminal").await;
    context.gateway.register_plugin(Arc::clone(&plugin)).await;

    // Build InboundChainInput from the chat message.
    let input = InboundChainInput {
        platform: "terminal".to_string(),
        sender_id: agent_id.clone(),
        peer_id: "cli".to_string(),
        content,
        message_id: format!("chat-{}", chrono::Utc::now().timestamp_millis()),
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        account_id: Some("owner".to_string()),
        thread_id: None,
        message_type: MessageType::Text,
        media_refs: vec![],
        chat_name: None,
        trace_id: Some(format!("chat-{}", chrono::Utc::now().timestamp_millis())),
    };

    // Run the inbound processor chain (RawLog → SessionRouter → ContentNormalizer).
    let processed = context.gateway.process_inbound_chain(&input).await;

    // Dispatch through Gateway: resolves session, routes to LLM or slash command.
    // The Gateway spawns async tasks for LLM calls; responses come through plugin.send().
    let gw = Arc::clone(&context.gateway);
    let sender_id = agent_id.clone();
    let mut handle = tokio::spawn(async move {
        gw.handle_inbound_message(processed, Some(&sender_id), "terminal")
            .await
    });

    // Collect responses from the channel until Done or channel closes.
    let mut responses = Vec::new();

    // Wait for either the Gateway to return or for streaming responses.
    // Use a select loop to read from the channel while the Gateway processes.
    loop {
        tokio::select! {
            rendered = rx.recv() => {
                match rendered {
                    Some(output) => {
                        let chat_resp = rendered_to_response(&output);
                        responses.push(chat_resp);
                    }
                    None => {
                        // Channel closed — plugin was dropped or send failed.
                        // This means the Gateway has finished processing.
                        break;
                    }
                }
            }
            result = &mut handle => {
                // Gateway handle returned — check if there are more responses.
                match result {
                    Ok(Some(_)) => {
                        // Gateway finished, drain remaining channel messages.
                        while let Ok(output) = rx.try_recv() {
                            let chat_resp = rendered_to_response(&output);
                            responses.push(chat_resp);
                        }
                    }
                    Ok(None) => {
                        // Gateway returned None (message not processed).
                        while let Ok(output) = rx.try_recv() {
                            let chat_resp = rendered_to_response(&output);
                            responses.push(chat_resp);
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "chat message handler panicked");
                        responses.push(ChatResponse::Error {
                            message: format!("internal error: {}", e),
                        });
                    }
                }
                // Drain any remaining messages after handle completes.
                while let Ok(output) = rx.try_recv() {
                    let chat_resp = rendered_to_response(&output);
                    responses.push(chat_resp);
                }
                break;
            }
        }
    }

    // Restore the original terminal plugin so other connections are not affected.
    if let Some(original) = original_plugin {
        context.gateway.register_plugin(original).await;
    }

    // Append Done marker if we got any content.
    if !responses.is_empty() && !responses.iter().any(|r| matches!(r, ChatResponse::Done)) {
        responses.push(ChatResponse::Done);
    }

    // If no responses at all, send an error.
    if responses.is_empty() {
        responses.push(ChatResponse::Error {
            message: "no response from gateway".to_string(),
        });
    }

    responses
}

/// Convert a [`RenderedOutput`] to a [`ChatResponse`].
///
/// Extracts text content from the rendered output payload and wraps it
/// in the appropriate `ChatResponse` variant.
fn rendered_to_response(output: &RenderedOutput) -> ChatResponse {
    match output.msg_type.as_str() {
        "text" => {
            // Text payload: extract from {"content": {"text": "..."}} or raw string
            let text = extract_text_from_payload(&output.payload);
            ChatResponse::ContentChunk { text }
        }
        "interactive" => {
            // Interactive/card output: serialize as JSON string
            let text = serde_json::to_string(&output.payload)
                .unwrap_or_else(|_| output.payload.to_string());
            ChatResponse::ContentChunk { text }
        }
        other => {
            // Unknown type: best-effort text extraction
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
///
/// Handles both `{"content": {"text": "..."}}` (from send_text) and
/// raw string payloads.
fn extract_text_from_payload(payload: &serde_json::Value) -> String {
    // Try {"content": {"text": "..."}} first (standard Gateway format)
    if let Some(text) = payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
    {
        return text.to_string();
    }
    // Try raw string
    if let Some(text) = payload.as_str() {
        return text.to_string();
    }
    // Fallback: serialize the whole payload
    payload.to_string()
}

/// Handle a stop session request.
///
/// Sends a "/stop" command through the Gateway's inbound pipeline,
/// which is routed to the StopHandler slash command.
async fn dispatch_stop_session(agent_id: String, context: &ChatContext) -> Vec<ChatResponse> {
    // Build a /stop message and route it through the Gateway.
    let input = InboundChainInput {
        platform: "terminal".to_string(),
        sender_id: agent_id.clone(),
        peer_id: "cli".to_string(),
        content: "/stop".to_string(),
        message_id: format!("stop-{}", chrono::Utc::now().timestamp_millis()),
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        account_id: Some("owner".to_string()),
        thread_id: None,
        message_type: MessageType::Text,
        media_refs: vec![],
        chat_name: None,
        trace_id: None,
    };

    // Run through the inbound processor chain.
    let processed = context.gateway.process_inbound_chain(&input).await;

    // Dispatch through Gateway — the /stop command will be handled
    // by the SlashDispatcher → StopHandler.
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
/// and forward it through an [`mpsc`] channel to the Chat RPC connection
/// handler, which sends it over the Unix socket to the CLI client.
pub struct RpcTerminalPlugin {
    /// Channel sender for forwarding rendered output to the RPC handler.
    sender: tokio::sync::Mutex<Option<mpsc::Sender<RenderedOutput>>>,
    /// Streaming renderer for handling incremental LLM output.
    streaming_renderer: std::sync::Mutex<DefaultStreamingRenderer>,
}

impl RpcTerminalPlugin {
    /// Create a new RPC terminal plugin with the given channel sender.
    pub fn new(sender: mpsc::Sender<RenderedOutput>) -> Self {
        Self {
            sender: tokio::sync::Mutex::new(Some(sender)),
            streaming_renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
        }
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
        // as ChatRequest::ChatMessage, not as raw webhook payloads.
        Ok(None)
    }

    fn streaming_renderer(&self) -> Option<&std::sync::Mutex<DefaultStreamingRenderer>> {
        Some(&self.streaming_renderer)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        // Render content blocks to plain text (same as TerminalPlugin).
        let mut rendered = String::new();
        for (i, block) in content_blocks.iter().enumerate() {
            if let ContentBlock::Text(text) = block {
                if i > 0 {
                    rendered.push('\n');
                }
                rendered.push_str(text);
            }
        }
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String(rendered),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        // Forward the rendered output through the channel to the RPC handler.
        let sender = self.sender.lock().await;
        match sender.as_ref() {
            Some(tx) => tx
                .send(output.clone())
                .await
                .map_err(|e| AdapterError::SendFailed(e.to_string())),
            None => Err(AdapterError::SendFailed(
                "channel closed (plugin shut down)".to_string(),
            )),
        }
    }

    fn clean_content(&self, raw: &str) -> String {
        raw.to_string()
    }

    async fn init(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), AdapterError> {
        // Drop the sender to signal the RPC handler that the connection is done.
        let _ = self.sender.lock().await.take();
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
    use serde_json::json;

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
        let (tx, _rx) = mpsc::channel(1);
        let plugin = RpcTerminalPlugin::new(tx);
        let blocks = vec![ContentBlock::Text("line1".to_string())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        assert_eq!(output.payload, json!("line1"));
    }

    #[test]
    fn test_rpc_terminal_plugin_render_multiple_blocks() {
        let (tx, _rx) = mpsc::channel(1);
        let plugin = RpcTerminalPlugin::new(tx);
        let blocks = vec![
            ContentBlock::Text("line1".to_string()),
            ContentBlock::Text("line2".to_string()),
        ];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.payload, json!("line1\nline2"));
    }

    #[test]
    fn test_rpc_terminal_plugin_platform() {
        let (tx, _rx) = mpsc::channel(1);
        let plugin = RpcTerminalPlugin::new(tx);
        assert_eq!(plugin.platform(), "terminal");
    }

    #[tokio::test]
    async fn test_rpc_terminal_plugin_send() {
        let (tx, mut rx) = mpsc::channel(4);
        let plugin = RpcTerminalPlugin::new(tx);
        let output = RenderedOutput {
            msg_type: "text".to_string(),
            payload: json!("test message"),
        };
        plugin.send(&output, "peer", None).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received, output);
    }

    #[tokio::test]
    async fn test_rpc_terminal_plugin_shutdown_drops_sender() {
        let (tx, _rx) = mpsc::channel(4);
        let plugin = RpcTerminalPlugin::new(tx);
        plugin.shutdown().await.unwrap();
        // After shutdown, send should fail (sender dropped).
        let output = RenderedOutput {
            msg_type: "text".to_string(),
            payload: json!("test"),
        };
        let result = plugin.send(&output, "peer", None).await;
        assert!(result.is_err());
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
}
