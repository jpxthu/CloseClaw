//! Integration tests for the debug_log framework across modules.
//!
//! Verifies:
//! - trace_id propagation across the full message chain
//! - span_id parent-child derivation (LLM → tool execution, agent spawn)
//! - non-message event trace_id generation via `generate_trace_id`
//! - JSONL file format correctness and field completeness

use closeclaw_common::trace_id::generate_trace_id;
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel, TraceContext};
use closeclaw_llm::debug_log::{emit_llm_event, LlmDebugLogContext, LlmEmitEventParams};
use closeclaw_permission::debug_log::{
    emit_permission_event, PermissionDebugLogContext, PermissionEmitEventParams,
};
use closeclaw_processor_chain::debug_log::{
    emit_processor_chain_event, ProcessorChainDebugLogContext, ProcessorChainEmitEventParams,
};
use closeclaw_session::debug_log::{
    emit_session_event, SessionDebugLogContext, SessionEmitEventParams,
};
use closeclaw_slash::debug_log::{emit_slash_event, SlashDebugLogContext, SlashEmitEventParams};
use closeclaw_tools::debug_log::{emit_tool_event, ToolsDebugLogContext, ToolsEmitEventParams};
use tempfile::TempDir;

/// Helper: create a `DebugLog` instance backed by a temp directory.
async fn make_debug_log(dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.unwrap()
}

/// Helper: read all JSONL log events from the temp directory.
fn read_log_events(dir: &TempDir) -> Vec<closeclaw_debug_log::LogEvent> {
    let mut events = Vec::new();
    for entry in std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map(|ext| ext == "jsonl").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            for line in content.lines() {
                if !line.trim().is_empty() {
                    events.push(serde_json::from_str(line).unwrap());
                }
            }
        }
    }
    events
}

// ---------------------------------------------------------------------------
// Scenario 1: Message chain trace_id propagation
//
// Simulates: ProcessorChain → Session → LLM → Tools
// All events share the same trace_id.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_message_chain_trace_id_propagation() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "msg-chain-trace-001";
    let session_key = Some("sk-test-abc");

    // ProcessorChain: inbound start
    let ctx_pc = ProcessorChainDebugLogContext::new(Some(&debug_log), trace_id, session_key);
    emit_processor_chain_event(ProcessorChainEmitEventParams {
        ctx: ctx_pc,
        level: LogLevel::Info,
        source_module: "processor_chain",
        event_type: "chain.inbound",
        payload: serde_json::json!({"direction": "inbound"}),
        parent: None,
    });

    // Session: lookup
    let ctx_s = SessionDebugLogContext::new(Some(&debug_log), trace_id, session_key);
    emit_session_event(SessionEmitEventParams {
        ctx: ctx_s,
        level: LogLevel::Debug,
        source_module: "session",
        event_type: "session.lookup",
        payload: serde_json::json!({"matched": true}),
        parent: None,
    });

    // LLM: call start
    let ctx_llm = LlmDebugLogContext::new(Some(&debug_log), trace_id, session_key);
    emit_llm_event(LlmEmitEventParams {
        ctx: ctx_llm,
        level: LogLevel::Info,
        source_module: "llm",
        event_type: "llm.call.start",
        payload: serde_json::json!({"model": "test-model"}),
        parent: None,
    });

    // Tools: execution start
    let ctx_t = ToolsDebugLogContext::new(Some(&debug_log), trace_id, session_key);
    emit_tool_event(ToolsEmitEventParams {
        ctx: ctx_t,
        level: LogLevel::Info,
        source_module: "tools",
        event_type: "tool.execution.start",
        payload: serde_json::json!({"tool": "file_ops"}),
        parent: None,
    });

    // ProcessorChain: outbound start
    let ctx_pc2 = ProcessorChainDebugLogContext::new(Some(&debug_log), trace_id, session_key);
    emit_processor_chain_event(ProcessorChainEmitEventParams {
        ctx: ctx_pc2,
        level: LogLevel::Info,
        source_module: "processor_chain",
        event_type: "chain.outbound",
        payload: serde_json::json!({"direction": "outbound"}),
        parent: None,
    });

    // Allow async writes to complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 5, "expected 5 events in message chain");

    // All events share the same trace_id
    for e in &events {
        assert_eq!(e.trace_id, trace_id);
    }

    // Verify session_key propagated
    for e in &events {
        assert_eq!(e.session_key.as_deref(), Some("sk-test-abc"));
    }

    // Verify event types in order
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "chain.inbound",
            "session.lookup",
            "llm.call.start",
            "tool.execution.start",
            "chain.outbound",
        ]
    );

    // Verify source modules
    let modules: Vec<&str> = events.iter().map(|e| e.source_module.as_str()).collect();
    assert_eq!(
        modules,
        vec![
            "processor_chain",
            "session",
            "llm",
            "tools",
            "processor_chain",
        ]
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: span_id parent-child derivation
//
// LLM creates a root span, then derives a child span for tool execution.
// Verify parent_span_id correctly points to the LLM span.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_span_id_parent_child_derivation() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "span-parent-child-001";

    // Root span: LLM call start (emit creates internal span_id)
    let ctx_llm = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx: ctx_llm,
        level: LogLevel::Info,
        source_module: "llm",
        event_type: "llm.call.start",
        payload: serde_json::json!({"model": "test"}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Read back the root event to capture its actual span_id
    let root_events = read_log_events(&tmp);
    assert_eq!(root_events.len(), 1);
    let root_span_id = root_events[0].span_id.clone();

    // Child span: tool execution (parent = LLM root span)
    let tool_parent = TraceContext {
        trace_id: trace_id.to_string(),
        span_id: root_span_id.clone(),
        parent_span_id: String::new(),
    };
    emit_tool_event(ToolsEmitEventParams {
        ctx: ToolsDebugLogContext::new(Some(&debug_log), trace_id, None),
        level: LogLevel::Info,
        source_module: "tools",
        event_type: "tool.execution.start",
        payload: serde_json::json!({"tool": "bash"}),
        parent: Some(&tool_parent),
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Read back to get the child's span_id
    let two_events = read_log_events(&tmp);
    assert_eq!(two_events.len(), 2);
    let child_span_id = two_events[1].span_id.clone();

    // Grandchild span: nested call (parent = child span)
    let grandchild_parent = TraceContext {
        trace_id: trace_id.to_string(),
        span_id: child_span_id.clone(),
        parent_span_id: root_span_id.clone(),
    };
    let ctx_llm2 = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx: ctx_llm2,
        level: LogLevel::Debug,
        source_module: "llm",
        event_type: "llm.response",
        payload: serde_json::json!({"tokens": 128}),
        parent: Some(&grandchild_parent),
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 3);

    // All share the same trace_id
    for e in &events {
        assert_eq!(e.trace_id, trace_id);
    }

    // Root span has no parent
    assert!(events[0].parent_span_id.is_empty());

    // Child span's parent_span_id points to root span
    assert_eq!(events[1].parent_span_id, root_span_id);

    // Grandchild's parent_span_id points to child's span_id
    assert_eq!(events[2].parent_span_id, child_span_id);

    // All span_ids are unique
    let span_ids: Vec<&str> = events.iter().map(|e| e.span_id.as_str()).collect();
    assert_ne!(span_ids[0], span_ids[1]);
    assert_ne!(span_ids[1], span_ids[2]);
    assert_ne!(span_ids[0], span_ids[2]);
}

// ---------------------------------------------------------------------------
// Scenario 3: Non-message event trace_id generation
//
// Uses generate_trace_id() for background/scheduled events.
// Verify format: {module}_{timestamp_hex}_{random_hex}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_non_message_event_trace_id_generation() {
    let trace_id_tasks = generate_trace_id("tasks");
    let trace_id_daemon = generate_trace_id("daemon");

    // Verify format: {module}_{timestamp_hex}_{random_hex}
    let parts_tasks: Vec<&str> = trace_id_tasks.split('_').collect();
    assert!(
        parts_tasks.len() >= 3,
        "trace_id should have at least 3 parts separated by '_': {:?}",
        trace_id_tasks
    );
    assert_eq!(parts_tasks[0], "tasks");
    assert!(
        u64::from_str_radix(parts_tasks[1], 16).is_ok(),
        "timestamp part should be valid hex: {}",
        parts_tasks[1]
    );
    // Remaining parts after module and timestamp form the random hex
    let random_part = parts_tasks[2..].join("_");
    assert_eq!(random_part.len(), 32, "UUID v4 hex should be 32 chars");

    // Different modules produce different prefixes
    assert!(trace_id_tasks.starts_with("tasks_"));
    assert!(trace_id_daemon.starts_with("daemon_"));

    // Multiple calls produce unique IDs
    let id1 = generate_trace_id("tasks");
    let id2 = generate_trace_id("tasks");
    assert_ne!(id1, id2);

    // Non-message events with independent trace_ids can be logged
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;

    let ctx = ProcessorChainDebugLogContext::new(Some(&debug_log), &trace_id_tasks, None);
    emit_processor_chain_event(ProcessorChainEmitEventParams {
        ctx,
        level: LogLevel::Info,
        source_module: "tasks",
        event_type: "task.scheduled",
        payload: serde_json::json!({"task": "memory_mining"}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].trace_id, trace_id_tasks);
    assert!(events[0].trace_id.starts_with("tasks_"));
}

// ---------------------------------------------------------------------------
// Scenario 4: JSONL file format correctness
//
// Verify that events written to JSONL have all required fields,
// valid JSON structure, and correct types.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_jsonl_file_format_correctness() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "format-test-001";

    let ctx = SessionDebugLogContext::new(Some(&debug_log), trace_id, Some("sk-format"));
    emit_session_event(SessionEmitEventParams {
        ctx,
        level: LogLevel::Info,
        source_module: "session",
        event_type: "session.created",
        payload: serde_json::json!({"agent_id": "test-agent", "ttl": 3600}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 1);

    let e = &events[0];

    // Required fields present and non-empty
    assert!(!e.trace_id.is_empty());
    assert!(!e.span_id.is_empty());
    assert!(e.timestamp > 0);
    assert_eq!(e.level, LogLevel::Info);
    assert_eq!(e.source_module, "session");
    assert_eq!(e.event_type, "session.created");

    // Parent span_id empty for root
    assert!(e.parent_span_id.is_empty());

    // Session key present
    assert_eq!(e.session_key.as_deref(), Some("sk-format"));

    // Payload preserved
    assert_eq!(e.payload["agent_id"], "test-agent");
    assert_eq!(e.payload["ttl"], 3600);

    // Verify raw JSONL line is valid JSON with expected keys
    let jsonl_line = e.to_jsonl().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&jsonl_line).unwrap();
    assert!(parsed.is_object());
    assert!(parsed.get("trace_id").is_some());
    assert!(parsed.get("span_id").is_some());
    assert!(parsed.get("parent_span_id").is_some());
    assert!(parsed.get("timestamp").is_some());
    assert!(parsed.get("level").is_some());
    assert!(parsed.get("source_module").is_some());
    assert!(parsed.get("event_type").is_some());
    assert!(parsed.get("payload").is_some());
    assert!(parsed.get("session_key").is_some());
}

// ---------------------------------------------------------------------------
// Scenario 5: Cross-module events share trace_id but have unique span_ids
//
// Each module emits with the same trace_id but produces independent
// spans. Verify no span_id collisions.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cross_module_unique_span_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "cross-module-001";

    // Emit from every module with the same trace_id
    let ctx_s = SessionDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_session_event(SessionEmitEventParams {
        ctx: ctx_s,
        level: LogLevel::Info,
        source_module: "session",
        event_type: "session.lookup",
        payload: serde_json::json!({}),
        parent: None,
    });

    let ctx_llm = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx: ctx_llm,
        level: LogLevel::Info,
        source_module: "llm",
        event_type: "llm.call.start",
        payload: serde_json::json!({}),
        parent: None,
    });

    let ctx_t = ToolsDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_tool_event(ToolsEmitEventParams {
        ctx: ctx_t,
        level: LogLevel::Info,
        source_module: "tools",
        event_type: "tool.execution.start",
        payload: serde_json::json!({}),
        parent: None,
    });

    let ctx_slash = SlashDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_slash_event(SlashEmitEventParams {
        ctx: ctx_slash,
        level: LogLevel::Info,
        source_module: "slash",
        event_type: "slash.command",
        payload: serde_json::json!({}),
        parent: None,
    });

    let ctx_perm = PermissionDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_permission_event(PermissionEmitEventParams {
        ctx: ctx_perm,
        level: LogLevel::Info,
        source_module: "permission",
        event_type: "permission.check",
        payload: serde_json::json!({}),
        parent: None,
    });

    let ctx_pc = ProcessorChainDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_processor_chain_event(ProcessorChainEmitEventParams {
        ctx: ctx_pc,
        level: LogLevel::Info,
        source_module: "processor_chain",
        event_type: "chain.inbound",
        payload: serde_json::json!({}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 6);

    // All share the same trace_id
    for e in &events {
        assert_eq!(e.trace_id, trace_id);
    }

    // All span_ids are unique
    let span_ids: Vec<&str> = events.iter().map(|e| e.span_id.as_str()).collect();
    let mut unique = std::collections::HashSet::new();
    for sid in &span_ids {
        assert!(unique.insert(*sid), "duplicate span_id found: {}", sid);
    }

    // All are root spans (no parent)
    for e in &events {
        assert!(e.parent_span_id.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Scenario 6: Permission approval child span
//
// Permission check creates root span, approval flow derives child span.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_permission_approval_child_span() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "perm-approval-001";

    // Root: permission check (emit creates internal span_id)
    let ctx_perm = PermissionDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_permission_event(PermissionEmitEventParams {
        ctx: ctx_perm,
        level: LogLevel::Info,
        source_module: "permission",
        event_type: "permission.check",
        payload: serde_json::json!({"result": "pending_approval"}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Read back to get the root span_id
    let root_events = read_log_events(&tmp);
    assert_eq!(root_events.len(), 1);
    let perm_root_span_id = root_events[0].span_id.clone();

    // Child: approval flow triggered (parent = permission check span)
    let perm_root_ctx = TraceContext {
        trace_id: trace_id.to_string(),
        span_id: perm_root_span_id.clone(),
        parent_span_id: String::new(),
    };
    let ctx_perm2 = PermissionDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_permission_event(PermissionEmitEventParams {
        ctx: ctx_perm2,
        level: LogLevel::Warn,
        source_module: "permission",
        event_type: "permission.approval",
        payload: serde_json::json!({"action": "awaiting_user"}),
        parent: Some(&perm_root_ctx),
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 2);

    // Root has no parent
    assert!(events[0].parent_span_id.is_empty());

    // Child points to root
    assert_eq!(events[1].parent_span_id, perm_root_span_id);

    // Same trace_id
    assert_eq!(events[0].trace_id, events[1].trace_id);

    // Correct event types
    assert_eq!(events[0].event_type, "permission.check");
    assert_eq!(events[1].event_type, "permission.approval");
}

// ---------------------------------------------------------------------------
// Scenario 7: LLM retry + failure events (Warn/Error levels)
//
// Verify degraded warning and error level events are written correctly.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_llm_retry_and_failure_events() {
    let tmp = tempfile::tempdir().unwrap();
    let debug_log = make_debug_log(&tmp).await;
    let trace_id = "llm-retry-001";

    // LLM call start
    let ctx = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx,
        level: LogLevel::Info,
        source_module: "llm",
        event_type: "llm.call.start",
        payload: serde_json::json!({"model": "test"}),
        parent: None,
    });

    // Retry (Warn)
    let ctx = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx,
        level: LogLevel::Warn,
        source_module: "llm",
        event_type: "llm.retry",
        payload: serde_json::json!({"attempt": 1, "reason": "rate_limit"}),
        parent: None,
    });

    // Failure (Error)
    let ctx = LlmDebugLogContext::new(Some(&debug_log), trace_id, None);
    emit_llm_event(LlmEmitEventParams {
        ctx,
        level: LogLevel::Error,
        source_module: "llm",
        event_type: "llm.failure",
        payload: serde_json::json!({"attempts": 3, "error": "max_retries_exceeded"}),
        parent: None,
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_log_events(&tmp);
    assert_eq!(events.len(), 3);

    assert_eq!(events[0].level, LogLevel::Info);
    assert_eq!(events[1].level, LogLevel::Warn);
    assert_eq!(events[2].level, LogLevel::Error);

    assert_eq!(events[0].event_type, "llm.call.start");
    assert_eq!(events[1].event_type, "llm.retry");
    assert_eq!(events[2].event_type, "llm.failure");

    // All share trace_id
    for e in &events {
        assert_eq!(e.trace_id, trace_id);
    }
}
