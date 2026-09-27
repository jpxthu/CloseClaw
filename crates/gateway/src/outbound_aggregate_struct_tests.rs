//! Unit tests for the outbound aggregate structs introduced by the
//! 7-parameter aggregation refactor:
//!
//! - [`SendDebugCtx`](crate::outbound_helpers::SendDebugCtx) — shared emit
//!   context for `emit_feishu_send_event` / `emit_send_completed_log` (Step 1.1)
//! - [`SendOutboundIds`](SendOutboundIds) — owned trace identity pair
//!   for `Gateway::send_outbound` (Step 1.2)
//! - [`CheckpointMeta`](crate::outbound_helpers::CheckpointMeta) — owned
//!   checkpoint metadata for `Gateway::make_outbound_msg` (Step 1.3)
//!
//! Behavior dimensions covered (per plan Step 1.4):
//! 1. Happy path — every aggregate field reaches the original function bodies:
//!    event payload fields, `Message` fields, and the `SendOutcome` branches
//!    are unchanged after aggregation.
//! 2. Boundary values — `trace_id` / `session_key` / `parent` as `None` vs
//!    `Some` behave identically to the pre-aggregation call sites (aligns with
//!    the `Some` scenarios in debug_log_tests.rs
//!    `test_outbound_feishu_events_emitted`).
//! 3. Error path — with `Option` fields `None` the events keep their
//!    emit/skip semantics (None → skip, no panic) and sends still complete.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};

use crate::outbound::SendOutcome;
use crate::outbound_helpers::{CheckpointMeta, SendDebugCtx};
use crate::{Gateway, GatewayConfig, SendOutboundIds, Session, SessionManager};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Mock persistence — in-memory checkpoint store (single-session usage)
// ---------------------------------------------------------------------------

struct RecordingMockPersist {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
}

impl RecordingMockPersist {
    fn new() -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl PersistenceService for RecordingMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        *self.checkpoint.lock().await = Some(cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        *self.checkpoint.lock().await = None;
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .checkpoint
            .lock()
            .await
            .as_ref()
            .map(|cp| vec![cp.session_id.clone()])
            .unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Mock plugin — text rendering, optional send failure
// ---------------------------------------------------------------------------

struct AggMockPlugin {
    fail_send: std::sync::atomic::AtomicBool,
}

impl AggMockPlugin {
    fn new() -> Self {
        Self {
            fail_send: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn failing() -> Self {
        Self {
            fail_send: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl IMPlugin for AggMockPlugin {
    fn platform(&self) -> &str {
        "feishu"
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
        _dsl_result: Option<&DslParseResult>,
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
        if self.fail_send.load(std::sync::atomic::Ordering::SeqCst) {
            Err(AdapterError::SendFailed("simulated send failure".into()))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_config(name: &str) -> GatewayConfig {
    GatewayConfig {
        name: name.to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Gateway + feishu mock plugin (no checkpoint manager, optional debug log).
async fn setup_gw(
    temp_dir: &TempDir,
    with_debug_log: bool,
    plugin: AggMockPlugin,
) -> (Gateway, Arc<SessionManager>) {
    let config = make_config("agg-struct-tests");
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm));
    if with_debug_log {
        gw.set_debug_log(make_debug_log(temp_dir).await).await;
    }
    gw.register_plugin(Arc::new(plugin)).await;
    (gw, sm)
}

/// Gateway with checkpoint manager backed by the recording mock, feishu
/// plugin (no debug log so emit paths stay no-ops).
async fn setup_gw_with_persist(plugin: AggMockPlugin) -> (Gateway, Arc<RecordingMockPersist>) {
    let config = make_config("agg-struct-persist-tests");
    let persist = Arc::new(RecordingMockPersist::new());
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm)).with_checkpoint_manager(Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    ));
    gw.register_plugin(Arc::new(plugin)).await;
    (gw, persist)
}

/// Pre-create a session bound to chat "oc_chat" on the feishu channel.
async fn create_session(sm: &SessionManager, session_id: &str) {
    sm.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: "oc_chat".to_string(),
            channel: "feishu".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
}

/// Poll recorded checkpoints until `session_id` appears (max ~2s).
async fn wait_for_checkpoint(
    persist: &RecordingMockPersist,
    session_id: &str,
) -> SessionCheckpoint {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let snap = persist.checkpoint.lock().await.clone();
        if let Some(cp) = snap {
            if cp.session_id == session_id {
                return cp;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "checkpoint for {} not persisted in time",
            session_id
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Poll recorded checkpoints until the Sent-flow terminal state for
/// `session_id` is reached (max ~2s): the write-ahead op is ack-cleared
/// (no `pending_operations`) **and** the delivered assistant message is
/// recorded as sent. The success flow always produces both, so waiting on
/// the combined terminal state avoids racing the intermediate ack-clear
/// snapshot that precedes the message persist.
async fn wait_for_sent_flow_checkpoint(
    persist: &RecordingMockPersist,
    session_id: &str,
) -> SessionCheckpoint {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let snap = persist.checkpoint.lock().await.clone();
        if let Some(cp) = snap {
            if cp.session_id == session_id
                && cp.pending_operations.is_empty()
                && cp
                    .outbound_pending
                    .iter()
                    .any(|p| p.role.as_deref() == Some("assistant") && p.sent)
            {
                return cp;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sent-flow terminal checkpoint for {} not reached in time",
            session_id
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Poll recorded checkpoints until exactly one assistant pending message
/// exists (max ~2s), returning a clone of the checkpoint.
async fn wait_for_pending_message(
    persist: &RecordingMockPersist,
    session_id: &str,
) -> (SessionCheckpoint, closeclaw_common::PendingMessage) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let snap = persist.checkpoint.lock().await.clone();
        if let Some(cp) = snap {
            if cp.session_id == session_id {
                let assistants: Vec<_> = cp
                    .outbound_pending
                    .iter()
                    .filter(|p| p.role.as_deref() == Some("assistant"))
                    .collect();
                if assistants.len() == 1 {
                    return (cp.clone(), assistants[0].clone());
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "checkpoint for {} missing assistant pending message in time",
            session_id
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Poll the debug-log JSONL files under `dir` (max ~2s).
async fn read_events_with_timeout(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let mut events = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        for line in content.lines().filter(|l| !l.trim().is_empty()) {
                            if let Ok(evt) = closeclaw_debug_log::LogEvent::from_jsonl(line) {
                                events.push(evt);
                            }
                        }
                    }
                }
            }
        }
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn text_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(text.to_string())]
}

/// Expected `dsl_result` for the `::button[label:Yes;action:confirm]` line,
/// generated by the real `DslParser::parse` so the expectation matches its
/// actual output: `label` + `action` are kept as provided and `value` is
/// defaulted to an empty string.
fn dsl_result_json() -> String {
    let (result, _clean) =
        closeclaw_processor_chain::DslParser.parse("::button[label:Yes;action:confirm]\n");
    serde_json::to_string(&result).expect("DslParseResult serialization cannot fail")
}

// ---------------------------------------------------------------------------
// ① Happy path — aggregate fields reach the original function bodies
// ---------------------------------------------------------------------------

/// Full-flow happy path through `Gateway::send_outbound` with both aggregates
/// populated:
/// - `SendOutboundIds { trace_id, session_key }` must reach both
///   `SendDebugCtx` construction sites (`feishu.api.send` + `send.completed`):
///   both events carry the exact trace_id/session_key, their own
///   emitter-local source_module/event_type, and the per-emitter payload
///   fields.
/// - `CheckpointMeta` must reach `make_outbound_msg`: the checkpointed
///   assistant message carries platform / dsl_result / content_blocks.
#[tokio::test]
async fn test_aggregate_ids_and_meta_full_flow() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let (gw, sm) = setup_gw(&temp_dir, true, AggMockPlugin::new()).await;
    create_session(&sm, "sess-agg-happy").await;

    // Body with a DSL line: the default DslParser (registered by Gateway::new)
    // stores its parsed result in "dsl_result" metadata, so both aggregates
    // carry real values end-to-end.
    let body = "::button[label:Yes;action:confirm]\naggregate happy body";
    let trace_id = "trace-agg-happy-001";
    let session_key = "feishu:ou_sender:oc_chat";
    let result = gw
        .send_outbound(
            "sess-agg-happy",
            "feishu",
            body,
            text_blocks(body),
            SendOutboundIds {
                trace_id: Some(trace_id.to_string()),
                session_key: Some(session_key.to_string()),
            },
        )
        .await;
    assert!(matches!(result, Ok(SendOutcome::Sent)));

    // SendDebugCtx happy path: both aggregated emits fired with the struct's
    // identity fields, and each emitter kept its own payload fields.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = read_events_with_timeout(temp_dir.path()).await;

    let api_sends: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "feishu.api.send" && e.source_module == "feishu")
        .collect();
    assert_eq!(api_sends.len(), 1, "expected one feishu.api.send event");
    assert_eq!(api_sends[0].trace_id, trace_id);
    assert_eq!(api_sends[0].session_key.as_deref(), Some(session_key));
    assert_eq!(api_sends[0].payload["platform"], "feishu");
    assert_eq!(api_sends[0].payload["peer_id"], "oc_chat");
    assert!(api_sends[0].payload["send_duration_ms"].as_u64().is_some());

    let completed: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "send.completed" && e.source_module == "gateway")
        .collect();
    assert_eq!(completed.len(), 1, "expected one send.completed event");
    assert_eq!(completed[0].trace_id, trace_id);
    assert_eq!(completed[0].session_key.as_deref(), Some(session_key));
    assert_eq!(completed[0].payload["channel"], "feishu");
    assert_eq!(completed[0].payload["peer_id"], "oc_chat");

    // CheckpointMeta happy path: the checkpointed message mirrors the
    // aggregated metadata through make_outbound_msg → persist_outbound_checkpoint.
    let expected_dsl = dsl_result_json();
    let (gw2, persist) = setup_gw_with_persist(AggMockPlugin::new()).await;
    let sm2 = gw2.session_manager();
    create_session(&sm2, "sess-agg-happy-cp").await;
    let cp_body = "::button[label:Yes;action:confirm]\naggregate checkpoint body";
    let result = gw2
        .send_outbound(
            "sess-agg-happy-cp",
            "feishu",
            cp_body,
            text_blocks(cp_body),
            SendOutboundIds::default(),
        )
        .await;
    assert!(matches!(result, Ok(SendOutcome::Sent)));
    let (_cp, pending) = wait_for_pending_message(&persist, "sess-agg-happy-cp").await;
    assert_eq!(pending.platform.as_deref(), Some("feishu"));
    let got_dsl = pending.dsl_result.as_deref().expect("dsl_result persisted");
    let got: DslParseResult = serde_json::from_str(got_dsl).expect("persisted dsl_result parses");
    let expected: DslParseResult =
        serde_json::from_str(&expected_dsl).expect("expected dsl_result parses");
    assert_eq!(got, expected);
    let blocks_json = pending.content_blocks.as_deref().expect("blocks persisted");
    let got_blocks: Vec<ContentBlock> =
        serde_json::from_str(blocks_json).expect("persisted blocks parse");
    assert!(
        got_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("aggregate checkpoint body"))),
        "content_blocks must carry the processed text blocks"
    );
}

/// Happy path for `SendOutcome` branches under aggregation: a successful
/// plugin.send maps to `Sent` (write-ahead op ack-cleared, message recorded),
/// a failed plugin.send maps to `Notified` (write-ahead op retained — the
/// recovery service owns it, no assistant message recorded because the
/// message was NOT delivered). Neither branch changes with the aggregated
/// parameters.
#[tokio::test]
async fn test_send_outcome_branches_with_aggregates() {
    // Sent branch.
    let (gw_ok, persist_ok) = setup_gw_with_persist(AggMockPlugin::new()).await;
    let sm_ok = gw_ok.session_manager();
    create_session(&sm_ok, "sess-outcome-sent").await;
    let outcome = gw_ok
        .send_outbound(
            "sess-outcome-sent",
            "feishu",
            "ok body",
            text_blocks("ok body"),
            SendOutboundIds {
                trace_id: Some("trace-outcome-sent".to_string()),
                session_key: Some("feishu:u:chat".to_string()),
            },
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Sent)));
    let cp = wait_for_sent_flow_checkpoint(&persist_ok, "sess-outcome-sent").await;
    assert!(
        cp.pending_operations.is_empty(),
        "write-ahead op must be ack-cleared after Sent"
    );
    assert!(
        cp.outbound_pending
            .iter()
            .any(|p| p.role.as_deref() == Some("assistant") && p.sent),
        "delivered assistant message must be recorded as sent"
    );

    // Notified branch.
    let (gw_fail, persist_fail) = setup_gw_with_persist(AggMockPlugin::failing()).await;
    let sm_fail = gw_fail.session_manager();
    create_session(&sm_fail, "sess-outcome-notified").await;
    let outcome = gw_fail
        .send_outbound(
            "sess-outcome-notified",
            "feishu",
            "body that fails to send",
            text_blocks("body that fails to send"),
            SendOutboundIds {
                trace_id: Some("trace-outcome-fail".to_string()),
                session_key: Some("feishu:u:chat".to_string()),
            },
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Notified)));
    let cp = wait_for_checkpoint(&persist_fail, "sess-outcome-notified").await;
    assert!(
        cp.outbound_pending
            .iter()
            .all(|p| p.role.as_deref() != Some("assistant")),
        "failed message must not be recorded as a delivered assistant message"
    );
    assert_eq!(
        cp.pending_operations.len(),
        1,
        "write-ahead op must be retained for recovery after Notified"
    );
    assert_eq!(
        cp.pending_operations[0].op_type,
        closeclaw_session::persistence::PendingOperationType::OutboundMessage
    );
}

// ---------------------------------------------------------------------------
// ② Boundary values — None vs Some identity inputs are equivalent
// ---------------------------------------------------------------------------

/// `SendOutboundIds` boundary: with all identity fields `None` the send flow
/// is identical to the `Some` case except that every debug emit is skipped.
/// Checkpoint behavior (message + platform) is unchanged either way.
#[tokio::test]
async fn test_send_outbound_ids_none_vs_some_boundary() {
    // None case: send completes, no debug events at all.
    let temp_none = TempDir::new().expect("TempDir::new failed");
    let (gw_none, persist_none) = setup_gw_with_persist_and_debug(
        &temp_none,
        AggMockPlugin::new(),
        "agg-none-case",
        "sess-agg-none",
    )
    .await;
    let outcome = gw_none
        .send_outbound(
            "sess-agg-none",
            "feishu",
            "boundary none body",
            text_blocks("boundary none body"),
            SendOutboundIds::default(),
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Sent)));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events_none = read_events_with_timeout(temp_none.path()).await;
    assert!(
        events_none
            .iter()
            .all(|e| e.event_type != "feishu.api.send" && e.event_type != "send.completed"),
        "None identity must skip both aggregated emits"
    );
    let (_cp_none, pending_none) = wait_for_pending_message(&persist_none, "sess-agg-none").await;
    assert_eq!(pending_none.platform.as_deref(), Some("feishu"));
    assert_eq!(pending_none.content, "boundary none body");

    // Some case: identical flow emits exactly one of each event.
    let temp_some = TempDir::new().expect("TempDir::new failed");
    let (gw_some, persist_some) = setup_gw_with_persist_and_debug(
        &temp_some,
        AggMockPlugin::new(),
        "agg-some-case",
        "sess-agg-some",
    )
    .await;
    let outcome = gw_some
        .send_outbound(
            "sess-agg-some",
            "feishu",
            "boundary some body",
            text_blocks("boundary some body"),
            SendOutboundIds {
                trace_id: Some("trace-agg-some".to_string()),
                session_key: Some("feishu:ou_sender:oc_chat".to_string()),
            },
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Sent)));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events_some = read_events_with_timeout(temp_some.path()).await;
    assert_eq!(
        events_some
            .iter()
            .filter(|e| e.event_type == "feishu.api.send")
            .count(),
        1
    );
    assert_eq!(
        events_some
            .iter()
            .filter(|e| e.event_type == "send.completed")
            .count(),
        1
    );
    let (_cp_some, pending_some) = wait_for_pending_message(&persist_some, "sess-agg-some").await;
    assert_eq!(pending_some.platform.as_deref(), Some("feishu"));
    assert_eq!(pending_some.content, "boundary some body");
}

/// Gateway with checkpoint manager + debug log sharing one temp dir,
/// session pre-created. Used by the None-vs-Some boundary test so both
/// variants observe identical pipeline state.
async fn setup_gw_with_persist_and_debug(
    temp_dir: &TempDir,
    plugin: AggMockPlugin,
    config_name: &str,
    session_id: &str,
) -> (Gateway, Arc<RecordingMockPersist>) {
    let config = make_config(config_name);
    let persist = Arc::new(RecordingMockPersist::new());
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm)).with_checkpoint_manager(Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    ));
    gw.set_debug_log(make_debug_log(temp_dir).await).await;
    gw.register_plugin(Arc::new(plugin)).await;
    create_session(&sm, session_id).await;
    (gw, persist)
}

/// `SendDebugCtx::trace_id_or_empty` is the shared empty-trace skip helper
/// used by `emit_feishu_send_event`: `None` maps to `""` (skip), `Some` maps
/// through unchanged (aligns with the pre-aggregation
/// `trace_id.as_deref().unwrap_or("")` call-site semantics).
#[test]
fn test_send_debug_ctx_trace_id_or_empty_matches_pre_aggregation() {
    let config = make_config("agg-trace-helper");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm));

    let ctx_none = SendDebugCtx {
        gateway: &gw,
        trace_id: None,
        session_key: None,
        parent: None,
    };
    assert_eq!(ctx_none.trace_id_or_empty(), "");
    assert_eq!(ctx_none.trace_id, None);
    assert_eq!(ctx_none.session_key, None);
    assert!(ctx_none.parent.is_none());

    let parent = closeclaw_debug_log::TraceContext::new_root("trace-agg-parent".to_string());
    let child = parent.child();
    let ctx_some = SendDebugCtx {
        gateway: &gw,
        trace_id: Some("trace-agg-some"),
        session_key: Some("feishu:ou_sender:oc_chat"),
        parent: Some(&child),
    };
    assert_eq!(ctx_some.trace_id_or_empty(), "trace-agg-some");
    assert_eq!(ctx_some.trace_id, Some("trace-agg-some"));
    assert_eq!(ctx_some.session_key, Some("feishu:ou_sender:oc_chat"));
    assert_eq!(
        ctx_some.parent.map(|p| p.span_id.as_str()),
        Some(child.span_id.as_str())
    );
}

/// `CheckpointMeta` boundary: `None` metadata yields a `Message` whose
/// optional fields are `None` (identical to the pre-aggregation literal
/// `None, None, None` call sites); `Some` values flow through unchanged.
#[test]
fn test_checkpoint_meta_none_vs_some_boundary() {
    let msg_none = Gateway::make_outbound_msg(
        "feishu",
        "oc_chat".to_string(),
        "out-agg-none".to_string(),
        "body".to_string(),
        CheckpointMeta {
            platform: None,
            dsl_result: None,
            content_blocks: None,
        },
    );
    assert_eq!(msg_none.platform, None);
    assert_eq!(msg_none.dsl_result, None);
    assert_eq!(msg_none.content_blocks, None);

    let msg_some = Gateway::make_outbound_msg(
        "feishu",
        "oc_chat".to_string(),
        "out-agg-some".to_string(),
        "body".to_string(),
        CheckpointMeta {
            platform: Some("feishu".to_string()),
            dsl_result: Some(dsl_result_json()),
            content_blocks: Some("blocks".to_string()),
        },
    );
    assert_eq!(msg_some.platform.as_deref(), Some("feishu"));
    // Parse instead of raw string equality: serde_json serializes HashMap
    // params in nondeterministic key order, so byte-level comparison of two
    // independent serializations can flake.
    let got: DslParseResult =
        serde_json::from_str(msg_some.dsl_result.as_deref().expect("dsl_result set"))
            .expect("dsl_result parses");
    let expected: DslParseResult =
        serde_json::from_str(&dsl_result_json()).expect("expected dsl_result parses");
    assert_eq!(got, expected);
    assert_eq!(msg_some.content_blocks.as_deref(), Some("blocks"));
}

// ---------------------------------------------------------------------------
// ③ Error path — None fields keep emit/skip semantics, sends still complete
// ---------------------------------------------------------------------------

/// Error-path combination: `trace_id: None` (emits skipped) while
/// `session_key: Some(...)` — the emit-skip must not swallow the send.
/// `send.completed`/`feishu.api.send` stay unemitted and the send completes.
#[tokio::test]
async fn test_none_trace_id_with_some_session_key_skip_and_complete() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let (gw, sm) = setup_gw(&temp_dir, true, AggMockPlugin::new()).await;
    create_session(&sm, "sess-agg-mixed").await;

    let outcome = gw
        .send_outbound(
            "sess-agg-mixed",
            "feishu",
            "mixed identity body",
            text_blocks("mixed identity body"),
            SendOutboundIds {
                trace_id: None,
                session_key: Some("feishu:ou_sender:oc_chat".to_string()),
            },
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Sent)));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = read_events_with_timeout(temp_dir.path()).await;
    assert!(
        events.iter().all(|e| e.event_type != "feishu.api.send"),
        "None trace_id must keep feishu.api.send skipped"
    );
    assert!(
        events.iter().all(|e| e.event_type != "send.completed"),
        "None trace_id must keep send.completed skipped"
    );
}

/// Incomplete metadata combinations (all-`None` and partial `Some`) are
/// accepted by `make_outbound_msg` — construction never fails and the
/// `Message` reflects exactly the provided subset.
#[test]
fn test_checkpoint_meta_partial_combinations() {
    let msg_partial = Gateway::make_outbound_msg(
        "feishu",
        "oc_chat".to_string(),
        "out-agg-partial".to_string(),
        "partial body".to_string(),
        CheckpointMeta {
            platform: Some("feishu".to_string()),
            dsl_result: None,
            content_blocks: None,
        },
    );
    assert_eq!(msg_partial.platform.as_deref(), Some("feishu"));
    assert_eq!(msg_partial.dsl_result, None);
    assert_eq!(msg_partial.content_blocks, None);
    assert_eq!(msg_partial.channel, "feishu");
    assert_eq!(msg_partial.to, "oc_chat");
    assert_eq!(msg_partial.content, "partial body");
    assert_eq!(msg_partial.from, "agent");
}

/// Failure path with debug log configured and `Some` identity: a failing
/// `plugin.send` still emits the aggregated `feishu.api.send` event (the
/// emit happens before the error branch, inside `send_and_record`) and the
/// `Notified` outcome preserves the error-path semantics.
#[tokio::test]
async fn test_send_failure_emits_aggregated_event_then_notifies() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let (gw, sm) = setup_gw(&temp_dir, true, AggMockPlugin::failing()).await;
    create_session(&sm, "sess-agg-fail").await;

    let outcome = gw
        .send_outbound(
            "sess-agg-fail",
            "feishu",
            "failing body",
            text_blocks("failing body"),
            SendOutboundIds {
                trace_id: Some("trace-agg-fail".to_string()),
                session_key: Some("feishu:ou_sender:oc_chat".to_string()),
            },
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Notified)));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = read_events_with_timeout(temp_dir.path()).await;
    let api_sends: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "feishu.api.send")
        .collect();
    assert_eq!(api_sends.len(), 1, "failure path must still emit once");
    assert_eq!(api_sends[0].trace_id, "trace-agg-fail");
    assert!(
        events.iter().all(|e| e.event_type != "send.completed"),
        "send.completed must not fire when the message was not delivered"
    );
}

/// Untouched send pipeline with fully-default aggregates keeps the
/// write-ahead semantics: the op is recorded before send with the target
/// channel, then ack-cleared after success and the message persisted as sent.
#[tokio::test]
async fn test_writeahead_op_lifecycle_with_default_aggregates() {
    let (gw, persist) = setup_gw_with_persist(AggMockPlugin::new()).await;
    let sm = gw.session_manager();
    create_session(&sm, "sess-agg-wa").await;

    let outcome = gw
        .send_outbound(
            "sess-agg-wa",
            "feishu",
            "writeahead body",
            text_blocks("writeahead body"),
            SendOutboundIds::default(),
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Sent)));

    let cp = wait_for_sent_flow_checkpoint(&persist, "sess-agg-wa").await;
    assert!(cp.pending_operations.is_empty());
    let assistant: Vec<_> = cp
        .outbound_pending
        .iter()
        .filter(|p| p.role.as_deref() == Some("assistant"))
        .collect();
    assert_eq!(assistant.len(), 1);
    assert!(assistant[0].sent, "persisted message must be marked sent");
}

/// `PendingOperation` write-ahead snapshot carries the aggregated dispatch
/// context's channel so recovery can attribute the op (`target_channel`).
#[tokio::test]
async fn test_writeahead_snapshot_carries_aggregated_channel() {
    let (gw, persist) = setup_gw_with_persist(AggMockPlugin::failing()).await;
    let sm = gw.session_manager();
    create_session(&sm, "sess-agg-wa-detail").await;

    let outcome = gw
        .send_outbound(
            "sess-agg-wa-detail",
            "feishu",
            "writeahead detail body",
            text_blocks("writeahead detail body"),
            SendOutboundIds::default(),
        )
        .await;
    assert!(matches!(outcome, Ok(SendOutcome::Notified)));

    let cp = wait_for_checkpoint(&persist, "sess-agg-wa-detail").await;
    assert_eq!(cp.pending_operations.len(), 1);
    let op = &cp.pending_operations[0];
    assert_eq!(
        op.op_type,
        closeclaw_session::persistence::PendingOperationType::OutboundMessage
    );
    assert_eq!(
        op.status,
        closeclaw_session::persistence::PendingOperationStatus::Running
    );
    match &op.detail {
        closeclaw_session::persistence::PendingOperationDetail::OutboundMessage {
            target_channel,
            ..
        } => assert_eq!(target_channel, "feishu"),
        other => panic!("unexpected pending op detail: {other:?}"),
    }
}
