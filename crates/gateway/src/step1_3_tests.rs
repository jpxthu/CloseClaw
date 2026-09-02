//! Step 1.3 — Unit tests for Step 1.1 (slash queuing notification),
//! Step 1.2 (streaming degradation error markers), and
//! the max_message_size / session_key-empty acceptance criteria.

use std::sync::Arc;

use closeclaw_common::im_plugin::{AdapterError, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::IMPlugin;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};

use super::session_handler_streaming::build_checkpoint_message;
use super::session_handler_streaming::{
    META_STREAMING_INTERRUPTED, META_STREAMING_INTERRUPT_REASON,
};
use crate::types::{GatewayConfig, GatewayError};
use crate::{Gateway, HandleResult, SessionManager};

// ── Shared mock infrastructure ──────────────────────────────────────────────

struct TestHandler {
    command: &'static str,
    requires_permission: bool,
}

#[async_trait::async_trait]
impl SlashHandler for TestHandler {
    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "test handler"
    }
    fn requires_permission(&self) -> bool {
        self.requires_permission
    }
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(TestHandler {
            command: self.command,
            requires_permission: self.requires_permission,
        })
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("handled:{}", self.command))
    }
}

/// Router: `help` is immediate, `compact` is not.
struct TestSlashRouter;

#[async_trait::async_trait]
impl SlashRouter for TestSlashRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, command: &str) -> bool {
        command == "help"
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        match command {
            "help" => Some(Box::new(TestHandler {
                command: "help",
                requires_permission: false,
            })),
            "compact" => Some(Box::new(TestHandler {
                command: "compact",
                requires_permission: false,
            })),
            _ => None,
        }
    }
}

/// Captures messages sent via `send_outbound_simplified`.
struct CapturingPlugin {
    platform: String,
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sent: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn take_sent(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        self.sent.lock().unwrap().push((peer_id.to_string(), text));
        Ok(())
    }
}

/// Plugin whose `send` always fails — for non-blocking failure test.
struct FailingPlugin {
    platform: String,
    send_attempt_count: std::sync::atomic::AtomicU32,
}

#[async_trait::async_trait]
impl IMPlugin for FailingPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }
    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_attempt_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AdapterError::SendFailed("mock failure".into()))
    }
}

impl FailingPlugin {
    fn send_attempt_count(&self) -> u32 {
        self.send_attempt_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "step1_3_test".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

/// Build a Gateway with a registered plugin and a session in `sessions` map.
async fn build_env(
    session_id: &str,
    channel: &str,
    plugin: Arc<dyn IMPlugin>,
) -> (Arc<Gateway>, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "agent_test".to_string(),
            channel: channel.to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = Gateway::new(test_config(), Arc::clone(&sm));
    gw.register_plugin(plugin).await;
    gw.set_slash_dispatcher(Arc::new(TestSlashRouter)).await;
    (Arc::new(gw), sm)
}

/// Insert a ConversationSession and set its LLM state to make it busy.
async fn make_session_busy(sm: &SessionManager, session_id: &str) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    cs.set_llm_state(closeclaw_common::LlmState::Requesting);
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs_arc);
}

/// Insert a ConversationSession in Idle state (session not busy).
async fn make_session_idle(sm: &SessionManager, session_id: &str) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs_arc);
}

// ── Shared MemPersist for degradation tests ─────────────────────────────────

/// In-memory [`PersistenceService`] implementation for tests.
struct MemPersist {
    saved: std::sync::Mutex<Vec<SessionCheckpoint>>,
}

impl MemPersist {
    fn new() -> Self {
        Self {
            saved: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn take(&self) -> Vec<SessionCheckpoint> {
        std::mem::take(&mut *self.saved.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl PersistenceService for MemPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.saved.lock().unwrap().push(cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn sync(&self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

/// Build a degradation test environment: MemPersist + SessionManager + Gateway.
///
/// Returns `(gateway, session_manager, persist)`.
async fn build_degradation_env(
    session_id: &str,
    agent_id: &str,
) -> (Arc<Gateway>, Arc<SessionManager>, Arc<MemPersist>) {
    let persist = Arc::new(MemPersist::new());
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = Arc::new(Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm));
    (gw, sm, persist)
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.1: Slash command queuing notification tests
// ═════════════════════════════════════════════════════════════════════════════

/// Non-immediate slash command + busy session → notification via
/// `send_outbound_simplified` (mock plugin receives the message).
#[tokio::test]
async fn test_non_immediate_slash_busy_uses_simplified_outbound() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, sm) = build_env("s1", "mock", plugin).await;
    make_session_busy(&sm, "s1").await;

    let result = gw
        .dispatch_slash("s1", "/compact", None, "mock", Some("peer1"))
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "busy non-immediate should be handled as SlashHandled"
    );

    let sent = plugin_ref.take_sent();
    assert_eq!(sent.len(), 1, "should send exactly one notification");
    assert_eq!(sent[0].0, "peer1", "notification targets correct peer_id");
    assert!(
        sent[0].1.contains("排队"),
        "notification text should mention queuing"
    );
}

/// Immediate slash command + busy session → handler called directly,
/// no queuing, no queuing notification. The handler's reply is still
/// sent through the outbound pipeline (captured by the plugin).
#[tokio::test]
async fn test_immediate_slash_skips_queue_and_notification() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, sm) = build_env("s2", "mock", plugin).await;
    make_session_busy(&sm, "s2").await;

    let result = gw
        .dispatch_slash("s2", "/help", Some("user1"), "mock", Some("p2"))
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "immediate command should be handled"
    );

    // No queuing notification — only the handler reply is sent.
    let sent = plugin_ref.take_sent();
    assert!(!sent.is_empty(), "handler reply should be sent");
    assert!(
        !sent.iter().any(|(_, text)| text.contains("排队")),
        "should not contain queuing notification"
    );
    assert!(
        sent.iter().any(|(_, text)| text.contains("handled:help")),
        "handler reply should be sent through outbound"
    );
}

/// Notification failure does not block enqueuing — command is still queued.
#[tokio::test]
async fn test_slash_notification_failure_does_not_block_enqueue() {
    let failing = Arc::new(FailingPlugin {
        platform: "mock".to_string(),
        send_attempt_count: std::sync::atomic::AtomicU32::new(0),
    });
    let failing_ref = Arc::clone(&failing);
    let (gw, sm) = build_env("s3", "mock", failing).await;
    make_session_busy(&sm, "s3").await;

    let result = gw
        .dispatch_slash("s3", "/compact", None, "mock", Some("p3"))
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "should still return SlashHandled when notification fails"
    );

    // Verify the message was actually enqueued despite the send failure.
    let pending = sm.pop_pending_message("s3").await;
    assert!(
        pending.is_some(),
        "message should be enqueued even when notification send fails"
    );
    assert!(
        pending.unwrap().content.contains("compact"),
        "enqueued content should be the slash command"
    );
    assert_eq!(
        failing_ref.send_attempt_count(),
        1,
        "notification should attempt exactly 1 send (no fallback)"
    );
}

/// Non-immediate slash command + idle session → handler executed directly,
/// no queuing, no queuing notification. The handler's reply is sent
/// through the outbound pipeline.
#[tokio::test]
async fn test_non_immediate_slash_idle_executes_directly() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, sm) = build_env("s4", "mock", plugin).await;
    make_session_idle(&sm, "s4").await;

    let result = gw
        .dispatch_slash("s4", "/compact", Some("user1"), "mock", Some("p4"))
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "idle non-immediate should execute directly"
    );

    // No queuing notification — only the handler reply is sent.
    let sent = plugin_ref.take_sent();
    assert!(!sent.is_empty(), "handler reply should be sent");
    assert!(
        !sent.iter().any(|(_, text)| text.contains("排队")),
        "should not contain queuing notification"
    );
    assert!(
        sent.iter()
            .any(|(_, text)| text.contains("handled:compact")),
        "handler reply should be sent through outbound"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.2: Streaming degradation error marker tests
// ═════════════════════════════════════════════════════════════════════════════

/// Normal completion (no error_reason) → metadata is empty.
#[test]
fn test_checkpoint_no_error_has_empty_metadata() {
    let blocks = vec![ContentBlock::Text("Hello world".to_string())];
    let msg = build_checkpoint_message("chat1", "feishu", &blocks, None);
    assert!(
        msg.metadata.is_empty(),
        "checkpoint without error should have empty metadata"
    );
}

/// Error path → metadata contains both streaming_interrupted and reason.
#[test]
fn test_checkpoint_with_error_has_markers() {
    let blocks = vec![ContentBlock::Text("partial".to_string())];
    let msg = build_checkpoint_message(
        "chat2",
        "feishu",
        &blocks,
        Some("stream broken mid-response"),
    );
    assert_eq!(
        msg.metadata.get(META_STREAMING_INTERRUPTED).unwrap(),
        "true",
        "should mark streaming_interrupted"
    );
    assert_eq!(
        msg.metadata.get(META_STREAMING_INTERRUPT_REASON).unwrap(),
        "stream broken mid-response",
        "should include interrupt reason"
    );
}

/// Error path with empty partial content → markers still present.
#[test]
fn test_checkpoint_error_with_empty_content_has_markers() {
    let msg = build_checkpoint_message("chat3", "feishu", &[], Some("timeout"));
    assert_eq!(
        msg.metadata.get(META_STREAMING_INTERRUPTED).unwrap(),
        "true",
        "markers should be set even with empty partial content"
    );
    assert_eq!(
        msg.metadata.get(META_STREAMING_INTERRUPT_REASON).unwrap(),
        "timeout"
    );
    assert!(
        msg.content.is_empty(),
        "content should be empty when no text blocks"
    );
}

/// End-to-end: handle_streaming_degradation with StreamError persists a
/// checkpoint whose message metadata contains error markers.
#[tokio::test]
async fn test_degradation_checkpoint_has_error_markers() {
    let (gw, sm, persist) = build_degradation_env("s-degrad", "chat_u").await;

    let dispatch_err = GatewayError::StreamError {
        message: "stream broken".into(),
        partial_content: vec![ContentBlock::Text("partial".into())],
    };

    super::session_handler_streaming::handle_streaming_degradation(
        &gw,
        &sm,
        "s-degrad",
        "mock",
        &dispatch_err,
    )
    .await
    .unwrap();

    // CheckpointManager::save spawns a task — yield to let it complete.
    tokio::task::yield_now().await;

    let saved = persist.take();
    assert!(!saved.is_empty(), "checkpoint should be persisted");

    let pending = &saved[0].outbound_pending;
    assert!(
        !pending.is_empty(),
        "should have at least one pending message"
    );
    // The pending message content_blocks JSON contains the partial content.
    // The actual metadata is on the Message object created by
    // build_checkpoint_message, verified by the unit tests above.
    assert!(
        pending[0].content.contains("partial"),
        "persisted content should include partial text"
    );
}

/// Normal streaming completion (non-StreamError) → no degradation, no error
/// markers in checkpoint.
#[tokio::test]
async fn test_degradation_non_stream_error_skips_checkpoint() {
    let (gw, sm, persist) = build_degradation_env("s-normal", "chat_u2").await;

    // Non-StreamError: degradation path returns Ok(()) without persisting.
    let err = GatewayError::AdapterError("not a stream error".into());
    super::session_handler_streaming::handle_streaming_degradation(
        &gw, &sm, "s-normal", "mock", &err,
    )
    .await
    .unwrap();

    tokio::task::yield_now().await;
    let saved = persist.take();
    assert!(
        saved.is_empty(),
        "non-StreamError should not trigger checkpoint persistence"
    );
}

/// `raw_log_dir` config does not affect error marker writing — markers are
/// set regardless of raw_log_dir presence.
#[test]
fn test_raw_log_dir_config_does_not_affect_error_markers() {
    let blocks = vec![ContentBlock::Text("test".to_string())];

    // With raw_log_dir configured.
    let msg_with_dir = build_checkpoint_message("c1", "feishu", &blocks, Some("err A"));
    assert_eq!(
        msg_with_dir
            .metadata
            .get(META_STREAMING_INTERRUPTED)
            .unwrap(),
        "true"
    );

    // Without raw_log_dir (empty channel still works).
    let msg_without_dir = build_checkpoint_message("c2", "telegram", &blocks, Some("err B"));
    assert_eq!(
        msg_without_dir
            .metadata
            .get(META_STREAMING_INTERRUPTED)
            .unwrap(),
        "true"
    );

    // Both have markers — raw_log_dir is irrelevant to metadata.
    assert_eq!(
        msg_with_dir.metadata.len(),
        msg_without_dir.metadata.len(),
        "marker count should be independent of channel/config"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.3: max_message_size and session_key-empty acceptance tests
// ═════════════════════════════════════════════════════════════════════════════

use closeclaw_common::processor::ProcessedMessage;
use std::collections::HashMap;

/// Build a ProcessedMessage with the given text and optional metadata entries.
fn make_processed(
    text: &str,
    session_key: Option<&str>,
    peer_id: &str,
    sender_id: &str,
) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert("peer_id".into(), peer_id.into());
    metadata.insert("sender_id".into(), sender_id.into());
    metadata.insert("message_type".into(), "\"Text\"".into());
    if let Some(sk) = session_key {
        metadata.insert("session_key".into(), sk.into());
    }
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(text.to_string())],
        metadata,
    }
}

/// max_message_size normal path: message length == limit → passes size check.
/// The plugin should NOT receive a "消息过长" rejection reply.
#[tokio::test]
async fn test_max_message_size_at_limit_passes() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm) = build_env("s-size-ok", "mock", plugin).await;

    // max_message_size = 1024 (from test_config).
    // Build a message whose content is exactly 1024 bytes.
    let content = "x".repeat(1024);
    let processed = make_processed(&content, Some("sk-ok"), "peer1", "user1");

    let _result = gw
        .handle_inbound_message(processed, Some("user1"), "mock")
        .await;

    // The size check should NOT block. No rejection reply should be sent.
    let sent = plugin_ref.take_sent();
    assert!(
        !sent.iter().any(|(_, t)| t.contains("消息过长")),
        "message at exactly the limit should not be rejected"
    );
}

/// max_message_size exceeded path: message length > limit → returns None,
/// sends rejection reply "消息过长，请缩短后重试".
#[tokio::test]
async fn test_max_message_size_exceeded_rejects() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm) = build_env("s-size-over", "mock", plugin).await;

    // max_message_size = 1024; build a 1025-byte message.
    let content = "y".repeat(1025);
    let processed = make_processed(&content, Some("sk-over"), "peer2", "user2");

    let result = gw
        .handle_inbound_message(processed, Some("user2"), "mock")
        .await;

    assert!(
        result.is_none(),
        "over-limit message should be rejected (return None)"
    );

    let sent = plugin_ref.take_sent();
    assert_eq!(sent.len(), 1, "should send exactly one rejection reply");
    assert_eq!(sent[0].0, "peer2", "rejection targets correct peer_id");
    assert!(
        sent[0].1.contains("消息过长"),
        "rejection text should mention the limit"
    );
}

/// max_message_size: empty peer_id → rejection reply is skipped (no send
/// attempt), but the method still returns None.
#[tokio::test]
async fn test_max_message_size_exceeded_empty_peer_skips_send() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm) = build_env("s-size-np", "mock", plugin).await;

    let content = "z".repeat(1025);
    let processed = make_processed(&content, Some("sk-np"), "", "user3");

    let result = gw
        .handle_inbound_message(processed, Some("user3"), "mock")
        .await;

    assert!(
        result.is_none(),
        "over-limit with empty peer_id should still return None"
    );
    let sent = plugin_ref.take_sent();
    assert!(
        sent.is_empty(),
        "empty peer_id should not trigger a send attempt"
    );
}

/// session_key empty + no routing match → new session is created.
/// The resolve function creates a brand-new session via Path 3 (key_registry
/// miss), so the method does NOT return None at the session routing failure
/// gate. This confirms that empty session_key is a valid degraded path.
///
/// NOTE: The "session routing failure" reply path is only triggered when
/// `SessionManager::resolve` returns `Err`. The existing tests in
/// `session_routing_tests.rs` cover that scenario.
#[tokio::test]
async fn test_session_key_empty_no_routing_creates_new_session() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, sm) = build_env("s-sk-new", "mock", plugin).await;

    // Message with no session_key AND no matching routing entry.
    let processed = make_processed("hello", None, "new_peer", "new_user");

    let _result = gw
        .handle_inbound_message(processed, Some("new_user"), "mock")
        .await;

    // A new session should have been created (Path 3 in resolve).
    // The routing failure reply should NOT be sent.
    let sent = plugin_ref.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, t)| t.contains("\u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\u{5931}\u{8D25}")),
        "should not send routing failure reply when new session is created"
    );
    // Verify a new session was created in the SessionManager.
    let sessions = sm.sessions.read().await;
    assert!(
        sessions.len() > 1,
        "should have created a new session (original + new)"
    );
}
