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
use std::time::Duration;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{
    AdapterError, IMPlugin, MessageType, NormalizedMessage, RenderedOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::streaming::DefaultStreamingRenderer;
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

/// Completion wait bound for LLM turns (mirrors the CLI REPL's
/// `STREAMING_TIMEOUT_SECS`). Prevents a connection handler from
/// blocking indefinitely if a turn never signals completion.
const TURN_COMPLETION_TIMEOUT_SECS: u64 = 120;

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
///
/// Synchronous results (slash replies, queued/error notifications) are
/// delivered inside the handler task, so a drain suffices. `LlmStarted`
/// means the LLM turn runs in a background dispatch task: its result is
/// delivered through the outbound pipeline onto this connection's
/// channel, and the daemon's turn-completion consumer closes the
/// channel when the turn finishes (see [`RpcTerminalPlugin::finish_turns`]).
/// Keep collecting until the channel closes, bounded by
/// [`TURN_COMPLETION_TIMEOUT_SECS`].
async fn collect_responses(
    rx: mpsc::Receiver<RenderedOutput>,
    handle: tokio::task::JoinHandle<Option<HandleResult>>,
) -> (Vec<ChatResponse>, mpsc::Receiver<RenderedOutput>) {
    collect_responses_with_timeout(
        rx,
        handle,
        Duration::from_secs(TURN_COMPLETION_TIMEOUT_SECS),
    )
    .await
}

/// [`collect_responses`] with an injectable completion-wait bound:
/// production passes [`TURN_COMPLETION_TIMEOUT_SECS`]; tests inject a
/// short bound to exercise the timeout branch deterministically.
///
/// While the handler task is pending, frames already on the channel
/// are drained concurrently (see [`await_handler_while_draining`]):
/// the connection channel holds at most 64 frames, so a synchronous
/// producer that fills it would otherwise block on `tx.send` forever —
/// the handler could never return, and a non-`LlmStarted` result has
/// no bounded wait that could recover from that.
async fn collect_responses_with_timeout(
    mut rx: mpsc::Receiver<RenderedOutput>,
    handle: tokio::task::JoinHandle<Option<HandleResult>>,
    turn_timeout: Duration,
) -> (Vec<ChatResponse>, mpsc::Receiver<RenderedOutput>) {
    let mut responses = Vec::new();
    // Phase 1 — drain while awaiting the handler; phase 2 — the
    // per-result collection below (bounded wait / trailing drain).
    let result = match await_handler_while_draining(&mut rx, handle, &mut responses).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "chat message handler panicked");
            responses.push(ChatResponse::Error {
                message: format!("internal error: {}", e),
            });
            None
        }
    };
    match result {
        Some(HandleResult::LlmStarted) => {
            // Frames collected in phase 1 are already in `responses`;
            // keep draining until the turn-completion consumer closes
            // the channel, bounded by `turn_timeout`.
            let wait = tokio::time::timeout(turn_timeout, async {
                while let Some(output) = rx.recv().await {
                    responses.push(rendered_to_response(&output));
                }
            })
            .await;
            if wait.is_err() {
                tracing::warn!("chat: timed out waiting for LLM turn completion");
            }
        }
        _ => drain_channel(&mut rx, &mut responses),
    }

    (responses, rx)
}

/// Await the handler task while concurrently draining the connection
/// channel: the `select!` races `rx.recv()` (each queued frame is
/// appended to `responses`) against the handle (breaks the loop with
/// the handler's result). A closed channel stops the recv arm — it
/// would return `None` immediately and spin — and the loop then only
/// awaits the handle.
///
/// Structural requirement: waiting for the handle *without* draining
/// deadlocks as soon as a synchronous producer fills the channel
/// (bounded at 64 frames): the producer blocks on `tx.send`, the
/// handler never returns, and a non-`LlmStarted` result would only
/// drain after `handle.await` — which can then never complete.
async fn await_handler_while_draining(
    rx: &mut mpsc::Receiver<RenderedOutput>,
    mut handle: tokio::task::JoinHandle<Option<HandleResult>>,
    responses: &mut Vec<ChatResponse>,
) -> Result<Option<HandleResult>, tokio::task::JoinError> {
    let mut is_channel_open = true;
    loop {
        if !is_channel_open {
            break handle.await;
        }
        tokio::select! {
            output = rx.recv() => match output {
                Some(output) => responses.push(rendered_to_response(&output)),
                None => is_channel_open = false,
            },
            result = &mut handle => break result,
        }
    }
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

/// Build a [`NormalizedMessage`] from the chat message content.
fn build_inbound_input(content: String) -> NormalizedMessage {
    let now_ms = chrono::Utc::now().timestamp_millis();
    NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: closeclaw_platform::current_uid(),
        peer_id: "cli".to_string(),
        content,
        timestamp: now_ms,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        chat_name: String::new(),
        trace_id: format!("chat-{}", now_ms),
        message_id: format!("chat-{}", now_ms),
        reply_ref: None,
        unavailable_media: vec![],
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
    let sender_id = input.sender_id.clone();
    let platform = input.platform.clone();
    // Run the inbound processor chain
    // (RawLog → SessionRouter → ContentNormalizer).
    let mut processed = context.gateway.process_inbound_chain(&input).await;
    // Design doc (cli/chat.md): the user names the target agent via
    // `--agent-id`; carry it on the processed message so Gateway session
    // resolution routes to that agent instead of the peer_id fallback.
    attach_target_agent(&mut processed, &agent_id);

    // Dispatch through Gateway: resolves session, routes to LLM or slash
    // command.
    let gw = Arc::clone(&context.gateway);
    let handle = tokio::spawn(CHAT_CONN_ID.scope(conn_id, async move {
        gw.handle_inbound_message(processed, Some(&sender_id), &platform)
            .await
    }));

    let (responses, _rx) = collect_responses(rx, handle).await;

    // Unregister the channel sender.
    context.rpc_plugin.unregister_sender(conn_id).await;

    finalize_responses(responses)
}

/// Attach the request's target `agent_id` to processed-message metadata.
///
/// Design doc (cli/chat.md / requirements cli §F1): "用户通过 --agent-id
/// 指定目标 agent". The Gateway's session resolution reads this key
/// (priority over bot→Agent bindings) so chat requests route to the
/// agent named in the request.
fn attach_target_agent(
    processed: &mut closeclaw_common::processor::ProcessedMessage,
    agent_id: &str,
) {
    processed
        .metadata
        .insert("agent_id".to_string(), agent_id.to_string());
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
    // Fallback route for LLM dispatch tasks: they lose the per-request
    // task-local, and `Gateway::send_outbound` addresses plugin sends by
    // the session's agent id (peer_id).
    context
        .rpc_plugin
        .register_agent_route(&agent_id, conn_id)
        .await;
    tracing::debug!(agent_id = %agent_id, "processing chat request for target agent");
    process_gateway_response(rx, conn_id, agent_id, content, context).await
}

/// Convert a [`RenderedOutput`] to a [`ChatResponse`].
fn rendered_to_response(output: &RenderedOutput) -> ChatResponse {
    match output.msg_type.as_str() {
        "text" => {
            let text = extract_text_from_payload(&output.payload);
            ChatResponse::ContentChunk { content: text }
        }
        "interactive" => {
            let text = serde_json::to_string(&output.payload)
                .unwrap_or_else(|_| output.payload.to_string());
            ChatResponse::ContentChunk { content: text }
        }
        other => {
            let text = extract_text_from_payload(&output.payload);
            if text.is_empty() {
                tracing::warn!(msg_type = other, "unknown RenderedOutput type");
                ChatResponse::ContentChunk {
                    content: output.payload.to_string(),
                }
            } else {
                ChatResponse::ContentChunk { content: text }
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

/// Build the `/stop` [`NormalizedMessage`] for a stop-session request.
fn build_stop_input(sender_id: String) -> NormalizedMessage {
    let now_ms = chrono::Utc::now().timestamp_millis();
    NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id,
        peer_id: "cli".to_string(),
        content: "/stop".to_string(),
        timestamp: now_ms,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: format!("stop-{}", now_ms),
        reply_ref: None,
        unavailable_media: vec![],
    }
}

/// Handle a stop session request.
///
/// Step 1.25 evidence: the stop chain **does** reach
/// [`RpcTerminalPlugin::send`] — `SlashResult::Stop` → `execute_stop`
/// (closeclaw-common executor) sends the "已停止当前任务" reply through
/// `route_slash_reply` → `Gateway::send_outbound`, keyed by the session's
/// agent id with no [`CHAT_CONN_ID`] task-local on this task. Registering
/// the per-request channel + agent route (symmetric with
/// [`dispatch_chat_message`]) gives that reply a destination, and the
/// frames it produced are surfaced to the stop requester before `Done`.
async fn dispatch_stop_session(agent_id: String, context: &ChatContext) -> Vec<ChatResponse> {
    let sender_id = closeclaw_platform::current_uid();
    let (mut rx, conn_id) = setup_rpc_channel(context).await;
    context
        .rpc_plugin
        .register_agent_route(&agent_id, conn_id)
        .await;

    let input = build_stop_input(sender_id.clone());
    let mut processed = context.gateway.process_inbound_chain(&input).await;
    // Route `/stop` to the same target agent as chat requests
    // (requirements cli §F1: `/stop` ends the current conversation).
    attach_target_agent(&mut processed, &agent_id);

    let mut responses = match context
        .gateway
        .handle_inbound_message(processed, Some(&sender_id), "terminal")
        .await
    {
        Some(_) => vec![ChatResponse::Done],
        None => vec![ChatResponse::Error {
            message: format!("failed to stop session for agent '{}'", agent_id),
        }],
    };

    // The stop chain's reply is fully buffered before the handler
    // returns (route_slash_reply awaits the send), so this drain is
    // deterministic — surface it ahead of the trailing Done/Error.
    let mut replies = Vec::new();
    while let Ok(output) = rx.try_recv() {
        replies.push(rendered_to_response(&output));
    }
    replies.append(&mut responses);
    context.rpc_plugin.unregister_sender(conn_id).await;
    replies
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
    /// Fallback route for sends that do not carry the request task's
    /// [`CHAT_CONN_ID`] task-local: agent_id → conn_id. LLM dispatch
    /// runs in a spawned task (task-locals do not propagate across
    /// `tokio::spawn`), so batch results delivered via
    /// `Gateway::send_outbound` reach `send()` without the task-local
    /// and are routed by the session's agent (the `peer_id` argument).
    /// Registered per chat request; latest registration wins for
    /// concurrent connections to the same agent.
    agent_routes: RwLock<HashMap<String, u64>>,
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
            agent_routes: RwLock::new(HashMap::new()),
            streaming_renderer: std::sync::Mutex::new(DefaultStreamingRenderer::new()),
            renderer: TerminalRenderer::new(),
        }
    }

    /// Register a sender for the given connection ID.
    pub async fn register_sender(&self, conn_id: u64, sender: mpsc::Sender<RenderedOutput>) {
        let mut conns = self.connections.write().await;
        conns.insert(conn_id, sender);
    }

    /// Register the agent → connection route used by sends that do not
    /// carry the [`CHAT_CONN_ID`] task-local (LLM dispatch tasks).
    pub async fn register_agent_route(&self, agent_id: &str, conn_id: u64) {
        let mut routes = self.agent_routes.write().await;
        routes.insert(agent_id.to_string(), conn_id);
    }

    /// Drop connection senders — `Some(conn_id)` drops just that
    /// connection, `None` drops all of them — together with the agent
    /// routes pointing at the dropped connections, so the two maps are
    /// always cleaned in pairs. Single implementation behind
    /// `unregister_sender`, `finish_turns` and `IMPlugin::shutdown`.
    ///
    /// Returns the number of senders dropped (`finish_turns` logs it).
    async fn clear_connections(&self, conn_id: Option<u64>) -> usize {
        let mut conns = self.connections.write().await;
        let dropped = match conn_id {
            Some(id) => usize::from(conns.remove(&id).is_some()),
            None => {
                let waiting = conns.len();
                conns.clear();
                waiting
            }
        };
        let mut routes = self.agent_routes.write().await;
        match conn_id {
            Some(id) => routes.retain(|_, c| *c != id),
            None => routes.clear(),
        }
        dropped
    }

    /// Unregister the sender for the given connection ID, along with any
    /// agent route pointing at it.
    pub async fn unregister_sender(&self, conn_id: u64) {
        self.clear_connections(Some(conn_id)).await;
    }

    /// Complete all in-flight chat turns: drop the registered connection
    /// senders so [`collect_responses`] observes channel close and
    /// finalizes the turn (content frames + `Done`).
    ///
    /// Called by the daemon's SessionMessageHandler output consumer,
    /// which receives one message per completed LLM turn. With a single
    /// active chat connection (the interactive CLI case) this finalizes
    /// exactly the turn that just completed; with concurrent LLM turns
    /// on multiple connections the first completion finalizes all
    /// waiting connections.
    pub async fn finish_turns(&self) {
        let waiting = self.clear_connections(None).await;
        if waiting > 0 {
            tracing::debug!(
                waiting,
                "chat: turn completed — closing waiting connections"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Turn-completion consumer
// ---------------------------------------------------------------------------

/// Spawn the turn-completion consumer for a `SessionMessageHandler`
/// output channel.
///
/// The handler emits one `(text, blocks)` message per completed LLM
/// turn (empty payload on failure); this consumer turns each message
/// into an [`RpcTerminalPlugin::finish_turns`] call so waiting chat
/// connections observe channel close and [`collect_responses`]
/// finalizes the turn instead of waiting out
/// [`TURN_COMPLETION_TIMEOUT_SECS`]. The loop exits when the output
/// channel closes (handler dropped).
///
/// Single assembly point for all consumers — startup path, restart
/// path and tests alike; the 120s-hang regression this guards against
/// was caused by the two production wirings drifting apart (the
/// restart path re-dropped the receiver), so no call site may
/// hand-roll its own consumer loop.
///
/// Returns a `JoinHandle<()>`: production call sites fire-and-forget
/// it, tests may `await` it to confirm the consumer exited after the
/// output channel closes.
pub fn spawn_turn_completion_consumer(
    output_rx: mpsc::Receiver<(String, Vec<ContentBlock>)>,
    plugin: Arc<RpcTerminalPlugin>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut output_rx = output_rx;
        while let Some((text, _blocks)) = output_rx.recv().await {
            tracing::debug!(
                turn_len = text.len(),
                "LLM turn completed — finalizing chat turns"
            );
            plugin.finish_turns().await;
        }
    })
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
        peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        // Route to the current task's connection channel. LLM dispatch
        // tasks do not carry the request task-local (task-locals do not
        // propagate across `tokio::spawn`), so fall back to the agent
        // route registered at dispatch time.
        let conn_id = match CHAT_CONN_ID.try_with(|id| *id) {
            Ok(id) => id,
            Err(_) => {
                let routes = self.agent_routes.read().await;
                match routes.get(peer_id) {
                    Some(conn) => *conn,
                    None => {
                        return Err(AdapterError::SendFailed(format!(
                            "no chat connection registered for peer '{}'",
                            peer_id
                        )));
                    }
                }
            }
        };
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
        // Clear senders and agent routes together (via
        // `clear_connections`). Steady state (once shutdown has
        // returned): both maps are empty, so a send resolves neither a
        // route nor a connection and fails with the plain "no chat
        // connection registered" error instead of the misleading
        // "route hit → connection not found". Transient window: `send()`
        // resolves the route and the connection under two separate
        // locks, so a send already past the route lookup when shutdown
        // ran can still observe route-hit → connections-cleared.
        self.clear_connections(None).await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Chat RPC server assembly (startup + restart)
// ---------------------------------------------------------------------------

/// Register a fresh [`RpcTerminalPlugin`] on `gateway` and spawn a
/// [`ChatRpcServer`] serving `sock_path`.
///
/// Single assembly point for the chat RPC wiring — the startup path
/// (`Daemon::init_phase_6_chat_rpc`) and the gateway-restart path
/// (`Daemon::start_chat_rpc_server`) both call this so the
/// register → context → serve sequence cannot drift between the two
/// (same pattern as [`spawn_turn_completion_consumer`]).
///
/// Returns the serve task's join handle (callers keep it so the next
/// restart can abort it) and the registered plugin (callers wire it into
/// turn completion).
pub(crate) async fn spawn_chat_rpc_server(
    gateway: &Arc<Gateway>,
    sock_path: &Path,
) -> (tokio::task::JoinHandle<()>, Arc<RpcTerminalPlugin>) {
    let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
    gateway
        .register_plugin(rpc_plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
        .await;
    let context = ChatContext {
        gateway: Arc::clone(gateway),
        rpc_plugin: rpc_plugin.clone(),
    };
    let chat_server = ChatRpcServer::new(sock_path, context);
    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_server.serve().await {
            tracing::error!(error = %e, "chat RPC server failed");
        }
    });
    tracing::info!("chat RPC server started on {}", sock_path.display());
    (chat_handle, rpc_plugin)
}

/// Handles returned by chat RPC init: the server task handle and the
/// socket path (the phase owns the registered terminal IM plugin and
/// wires it into the turn-completion consumer). Lives here (next to
/// [`spawn_chat_rpc_server`]) — the daemon struct keeps only state fields.
pub(crate) type ChatRpcInit = (tokio::task::JoinHandle<()>, PathBuf);

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
#[path = "chat_rpc_tests.rs"]
mod tests;
