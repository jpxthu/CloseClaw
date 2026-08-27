// ── Streaming error-handling unit tests ─────────────────────────────────────
//
// Verifies that `handle_stream_error` sends partial text blocks to the sink
// before the error message when a `StreamError` occurs mid-stream.
//
// Also covers `handle_streaming_degradation` integration tests for the
// StreamError degradation path (IM notification + checkpoint persistence).

use closeclaw_common::im_plugin::{AdapterError, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::{IMPlugin, StreamDone, StreamingSink};
use closeclaw_llm::ChatSession;
use closeclaw_llm::LLMError;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::sync::{Arc, Mutex};

use super::session_handler_streaming::{handle_stream_error, handle_streaming_degradation};
use crate::types::{GatewayConfig, GatewayError, Session};
use crate::{Gateway, SessionManager};

// ── Recording sink ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct RecordingSink {
    texts: Mutex<Vec<String>>,
    errors: Mutex<Vec<String>>,
    dones: Mutex<Vec<StreamDone>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            texts: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            dones: Mutex::new(Vec::new()),
        }
    }
}

impl StreamingSink for RecordingSink {
    fn send_text(&self, delta: &str) {
        self.texts.lock().unwrap().push(delta.to_string());
    }
    fn send_done(&self, done: StreamDone) {
        self.dones.lock().unwrap().push(done);
    }
    fn send_error(&self, error: String) {
        self.errors.lock().unwrap().push(error);
    }
}

// ── Mock persistence for degradation tests ──────────────────────────────────

/// Records every checkpoint save so tests can inspect what was persisted.
struct DegradMockPersist {
    saved: Arc<Mutex<Vec<SessionCheckpoint>>>,
}

impl DegradMockPersist {
    fn new() -> Self {
        Self {
            saved: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn take_saved(&self) -> Vec<SessionCheckpoint> {
        std::mem::take(&mut *self.saved.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl PersistenceService for DegradMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.saved.lock().unwrap().push(cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn sync(&self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

// ── Mock plugin for degradation tests ───────────────────────────────────────

/// Records calls to `send` so tests can verify notification parameters.
struct DegradMockPlugin {
    platform: String,
    sent: Arc<Mutex<Vec<(String, String)>>>, // (peer_id, rendered_text)
}

impl DegradMockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn take_sent(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl IMPlugin for DegradMockPlugin {
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
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
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

/// Plugin whose `send` always fails — used for non-blocking failure test.
struct FailingSendPlugin {
    platform: String,
}

#[async_trait::async_trait]
impl IMPlugin for FailingSendPlugin {
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
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
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
    ) -> Result<(), AdapterError> {
        Err(AdapterError::SendFailed("mock send failure".into()))
    }
}

// ── Degradation test helpers ────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-degrad".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

/// Build a Gateway + SessionManager pair for degradation tests.
///
/// Returns (gateway, session_manager, persist_ref, plugin).
/// The plugin is returned as `Arc<dyn IMPlugin>`; callers that need
/// to inspect sent messages should keep a typed reference separately.
async fn build_degradation_env(
    session_id: &str,
    chat_id: &str,
    channel: &str,
    plugin: Arc<dyn IMPlugin>,
) -> (
    Arc<Gateway>,
    Arc<SessionManager>,
    Arc<DegradMockPersist>,
    Arc<dyn IMPlugin>,
) {
    let persist = Arc::new(DegradMockPersist::new());
    let sm = Arc::new(SessionManager::new(
        &make_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: chat_id.to_string(),
            channel: channel.to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );

    let gw = Gateway::new(make_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);
    gw.register_plugin(Arc::clone(&plugin)).await;

    let gw_arc = Arc::new(gw);
    (gw_arc, sm, persist, plugin)
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// StreamError with two text blocks → both sent before error.
#[test]
fn test_stream_error_sends_partial_text_before_error() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Text("Hello, ".to_string()),
            ContentBlock::Text("world!".to_string()),
        ],
    };

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert_eq!(texts.len(), 2, "should have sent 2 text blocks");
    assert_eq!(texts[0], "Hello, ");
    assert_eq!(texts[1], "world!");

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: stream interrupted");
}

/// Error message is sent after all partial text blocks.
#[test]
fn test_stream_error_sends_error_after_partial_content() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "timeout".to_string(),
        partial_content: vec![ContentBlock::Text("partial".to_string())],
    };

    handle_stream_error(error, &sink);

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: timeout");
}

/// Mixed ContentBlocks (Text + Image) → only Text blocks sent, Image ignored.
#[test]
fn test_stream_error_only_text_blocks_sent_from_mixed_content() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Text("Hello, ".to_string()),
            ContentBlock::Image {
                name: "screenshot.png".to_string(),
                url: "https://example.com/img.png".to_string(),
            },
            ContentBlock::Text("world!".to_string()),
        ],
    };

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert_eq!(
        texts.len(),
        2,
        "should only send 2 text blocks, not the image block"
    );
    assert_eq!(texts[0], "Hello, ");
    assert_eq!(texts[1], "world!");

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
}

/// Empty partial_content → no text blocks sent, only the error.
#[test]
fn test_stream_error_empty_partial_content_sends_no_text() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "immediate failure".to_string(),
        partial_content: vec![],
    };

    handle_stream_error(error, &sink);

    let texts = sink.texts.lock().unwrap();
    assert!(
        texts.is_empty(),
        "no text blocks should be sent for empty partial_content"
    );

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: immediate failure");
}

/// Non-StreamError → only error message sent, no partial content.
#[test]
fn test_handle_stream_error_non_stream_error_sends_only_error() {
    let sink = RecordingSink::new();
    let error = GatewayError::MissingSessionId;

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert!(
        texts.is_empty(),
        "non-StreamError should not send any text blocks"
    );

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Missing session ID in message metadata");
}

/// StreamError with only non-text ContentBlocks → no text sent,
/// only the error.
#[test]
fn test_stream_error_non_text_blocks_sends_no_text() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Image {
                name: "screenshot.png".to_string(),
                url: "https://example.com/img.png".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "search".to_string(),
                input: r"{}".to_string(),
            },
        ],
    };

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert!(
        texts.is_empty(),
        "non-text blocks should not be sent as text"
    );

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: stream interrupted");
}

// ═════════════════════════════════════════════════════════════════════════════
// Degradation path tests (Step 1.5)
// ═════════════════════════════════════════════════════════════════════════════

/// Scenario 1: StreamError with partial_content → send_outbound_simplified
/// is called with the correct chat_id and error notification text.
#[tokio::test]
async fn test_degradation_sends_simplified_notification() {
    let plugin = Arc::new(DegradMockPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm, _persist, _plugin_arc) =
        build_degradation_env("s-degrad-1", "chat_user_1", "mock", plugin).await;

    let dispatch_err = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![ContentBlock::Text("Partial answer".to_string())],
    };

    let result = handle_streaming_degradation(&gw, &_sm, "s-degrad-1", "mock", &dispatch_err).await;
    assert!(
        result.is_ok(),
        "degradation should succeed, got {:?}",
        result
    );

    let sent = plugin_ref.take_sent();
    assert_eq!(sent.len(), 1, "should send exactly one notification");
    assert_eq!(
        sent[0].0, "chat_user_1",
        "notification targets correct chat_id"
    );
    assert!(
        sent[0].1.contains("回复中断"),
        "notification text should contain error message"
    );
}

/// Scenario 2: StreamError with partial_content → persist_outbound_checkpoint
/// is called with the partial content.
#[tokio::test]
async fn test_degradation_persists_checkpoint() {
    let plugin = Arc::new(DegradMockPlugin::new("mock"));
    let (gw, _sm, persist, _plugin_arc) =
        build_degradation_env("s-degrad-2", "chat_u2", "mock", plugin).await;

    let dispatch_err = GatewayError::StreamError {
        message: "timeout".to_string(),
        partial_content: vec![
            ContentBlock::Text("Line one\n".to_string()),
            ContentBlock::Text("Line two".to_string()),
        ],
    };

    handle_streaming_degradation(&gw, &_sm, "s-degrad-2", "mock", &dispatch_err)
        .await
        .unwrap();

    // CheckpointManager::save spawns a task for persistence — yield to let it
    // complete before inspecting the mock's saved vector.
    tokio::task::yield_now().await;

    let saved = persist.take_saved();
    assert!(
        !saved.is_empty(),
        "checkpoint should be persisted when partial_content is non-empty"
    );

    let cp = &saved[0];
    assert_eq!(cp.session_id, "s-degrad-2");
    assert!(
        !cp.outbound_pending.is_empty(),
        "checkpoint should have at least one pending message"
    );

    let pending = &cp.outbound_pending[0];
    assert!(pending.sent, "mark_sent should be true");
    assert!(
        pending.content.contains("Line one"),
        "pending content should include partial content"
    );
    assert!(
        pending.content.contains("Line two"),
        "pending content should include all partial text"
    );
}

/// Scenario 3: send_outbound_simplified fails → warn logged, but checkpoint
/// persistence still executes (non-blocking failure).
#[tokio::test]
async fn test_degradation_notification_failure_does_not_block_checkpoint() {
    let failing = Arc::new(FailingSendPlugin {
        platform: "mock".to_string(),
    });
    let (gw, _sm, persist, _plugin_arc) =
        build_degradation_env("s-degrad-3", "chat_u3", "mock", failing).await;

    let dispatch_err = GatewayError::StreamError {
        message: "network error".to_string(),
        partial_content: vec![ContentBlock::Text("important partial".to_string())],
    };

    let result = handle_streaming_degradation(&gw, &_sm, "s-degrad-3", "mock", &dispatch_err).await;
    assert!(
        result.is_ok(),
        "degradation should succeed even when notification fails, got {:?}",
        result
    );

    // CheckpointManager::save spawns a task — yield to let it complete.
    tokio::task::yield_now().await;

    let saved = persist.take_saved();
    assert_eq!(
        saved.len(),
        1,
        "checkpoint should still be persisted despite notification failure"
    );
    assert!(
        saved[0].outbound_pending[0]
            .content
            .contains("important partial"),
        "persisted partial content should be correct"
    );
}

/// Scenario 4: partial_content is empty → checkpoint persistence is skipped.
#[tokio::test]
async fn test_degradation_empty_partial_skips_checkpoint() {
    let plugin = Arc::new(DegradMockPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm, persist, _plugin_arc) =
        build_degradation_env("s-degrad-4", "chat_u4", "mock", plugin).await;

    let dispatch_err = GatewayError::StreamError {
        message: "immediate failure".to_string(),
        partial_content: vec![],
    };

    let result = handle_streaming_degradation(&gw, &_sm, "s-degrad-4", "mock", &dispatch_err).await;
    assert!(result.is_ok());

    // Notification is still sent even with empty partial_content.
    let sent = plugin_ref.take_sent();
    assert_eq!(sent.len(), 1, "notification should still be sent");

    // But no checkpoint should be persisted.
    let saved = persist.take_saved();
    assert!(
        saved.is_empty(),
        "empty partial_content should skip checkpoint persistence"
    );
}

/// Scenario 5: chat_id unavailable → degradation logic skips entirely
/// (no notification, no checkpoint). A warn log is emitted (tested via
/// Ok(()) return with no side effects).
#[tokio::test]
async fn test_degradation_chat_id_unavailable_skips() {
    let plugin = Arc::new(DegradMockPlugin::new("mock"));
    let plugin_ref = Arc::clone(&plugin);
    let (gw, _sm, persist, _plugin_arc) =
        build_degradation_env("s-degrad-5-unknown", "chat_u5", "mock", plugin).await;

    // Use a session_id that does NOT exist in SessionManager.
    let dispatch_err = GatewayError::StreamError {
        message: "stream broken".to_string(),
        partial_content: vec![ContentBlock::Text("lost data".to_string())],
    };

    let result =
        handle_streaming_degradation(&gw, &_sm, "nonexistent-session", "mock", &dispatch_err).await;
    assert!(
        result.is_ok(),
        "degradation should return Ok even when chat_id is missing"
    );

    // No notification sent.
    let sent = plugin_ref.take_sent();
    assert!(
        sent.is_empty(),
        "no notification should be sent when chat_id is unavailable"
    );

    // No checkpoint persisted.
    let saved = persist.take_saved();
    assert!(
        saved.is_empty(),
        "no checkpoint should be persisted when chat_id is unavailable"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.3: Streaming error Thinking block history retention tests
// ═════════════════════════════════════════════════════════════════════════════

// ── Behavior dimension 1 & 2: Thinking blocks preserved, Text filtered ─────

/// StreamError with Thinking → returns PartialContent; with Thinking + Text →
/// only Thinking preserved, Text sent to sink. Multiple Thinking all kept.
#[test]
fn test_stream_error_thinking_preserved_text_filtered() {
    let sink = RecordingSink::new();
    // Thinking + interleaved Text + ToolUse.
    let error = GatewayError::StreamError {
        message: "interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Thinking {
                thinking: "step 1".to_string(),
                signature: Some("sig".to_string()),
            },
            ContentBlock::Text("partial answer".to_string()),
            ContentBlock::Thinking {
                thinking: "step 2".to_string(),
                signature: None,
            },
            ContentBlock::ToolUse {
                id: "c1".to_string(),
                name: "exec".to_string(),
                input: r"{}".to_string(),
            },
        ],
    };
    let result = handle_stream_error(error, &sink);
    match result {
        LLMError::PartialContent {
            thinking_blocks, ..
        } => {
            assert_eq!(thinking_blocks.len(), 2, "both Thinking blocks preserved");
        }
        other => panic!(
            "expected PartialContent, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
    // Text sent to sink (user sees partial output) but not preserved.
    let texts = sink.texts.lock().unwrap();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0], "partial answer");
    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1, "error still sent");
}

// ── Behavior dimension 3: Edge cases ───────────────────────────────────────

/// Empty partial_content / Text-only / ToolUse-only → ApiError (no Thinking).
#[test]
fn test_stream_error_no_thinking_yields_api_error() {
    let cases: Vec<Vec<ContentBlock>> = vec![
        vec![],
        vec![ContentBlock::Text("partial".to_string())],
        vec![ContentBlock::ToolUse {
            id: "c1".to_string(),
            name: "exec".to_string(),
            input: r"{}".to_string(),
        }],
    ];
    for (i, partial_content) in cases.into_iter().enumerate() {
        let sink = RecordingSink::new();
        let error = GatewayError::StreamError {
            message: format!("case {i}"),
            partial_content,
        };
        let result = handle_stream_error(error, &sink);
        assert!(
            matches!(result, LLMError::ApiError(_)),
            "case {i}: no Thinking blocks should yield ApiError"
        );
    }
}

// ── Behavior dimension 4: Non-StreamError ───────────────────────────────────

/// Non-StreamError variants → ApiError, no partial content handling.
#[test]
fn test_non_stream_error_variants_return_api_error() {
    let errors = vec![
        GatewayError::MissingSessionId,
        GatewayError::AdapterError("plugin fail".to_string()),
    ];
    for error in errors {
        let sink = RecordingSink::new();
        let result = handle_stream_error(error, &sink);
        assert!(
            matches!(result, LLMError::ApiError(_)),
            "non-StreamError should return ApiError"
        );
        assert!(sink.texts.lock().unwrap().is_empty());
    }
}

// ── Behavior dimension 5: Context continuity ────────────────────────────────

/// Thinking-only message in history → orphaned by clean_thinking_content.
#[tokio::test]
async fn test_thinking_only_message_cleaned_from_api_request() {
    let mut session = ConversationSession::new(
        "s-think-clean".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    // User message via append_transcript.
    session.append_transcript("user", vec![ContentBlock::Text("What is 2+2?".to_string())]);
    // Streaming error: Thinking-only appended to history.
    session.append_response(closeclaw_llm::types::UnifiedResponse {
        content_blocks: vec![ContentBlock::Thinking {
            thinking: "Let me calculate...".to_string(),
            signature: None,
        }],
        usage: Default::default(),
        finish_reason: None,
        retry_attempts: 0,
    });
    assert_eq!(session.messages().len(), 2, "raw history has 2 messages");
    let request = session.build_api_request();
    // clean_thinking_content removes orphaned Thinking message.
    assert_eq!(request.messages.len(), 1, "orphaned Thinking removed");
    assert_eq!(request.messages[0].role, "user");
}

/// Normal assistant response with Thinking + Text → preserved (not error path).
/// clean_thinking_content keeps it because it has non-Thinking blocks.
#[tokio::test]
async fn test_normal_response_thinking_with_text_preserved() {
    let mut session = ConversationSession::new(
        "s-think-text".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    session.append_response(closeclaw_llm::types::UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Thinking {
                thinking: "reasoning".to_string(),
                signature: None,
            },
            ContentBlock::Text("the answer is 4".to_string()),
        ],
        usage: Default::default(),
        finish_reason: None,
        retry_attempts: 0,
    });
    let request = session.build_api_request();
    // Message preserved (has non-Thinking block).
    assert_eq!(request.messages.len(), 1, "message preserved (has Text)");
    assert!(
        request.messages[0].content.contains("the answer is 4"),
        "Text content should be present"
    );
}

/// Multiple orphaned Thinking messages all removed.
#[tokio::test]
async fn test_multiple_thinking_only_messages_all_cleaned() {
    let mut session = ConversationSession::new(
        "s-multi-think".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    session.append_transcript("user", vec![ContentBlock::Text("hello".to_string())]);
    // Two Thinking-only error interruptions.
    for thought in ["first", "second"] {
        session.append_response(closeclaw_llm::types::UnifiedResponse {
            content_blocks: vec![ContentBlock::Thinking {
                thinking: format!("{thought} thought"),
                signature: None,
            }],
            usage: Default::default(),
            finish_reason: None,
            retry_attempts: 0,
        });
    }
    assert_eq!(session.messages().len(), 3, "raw history has 3 messages");
    let request = session.build_api_request();
    assert_eq!(request.messages.len(), 1, "all orphaned Thinking removed");
    assert_eq!(request.messages[0].role, "user");
}
