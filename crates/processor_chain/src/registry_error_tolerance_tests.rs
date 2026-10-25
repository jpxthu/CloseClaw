//! Tests for [`ProcessorRegistry`] error tolerance — a failing processor must
//! not halt the chain (fail-open); subsequent processors still run, and the
//! fail-open handler logs `error!` for `raw_log` failures, `warn!` otherwise.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tracing_test::traced_test;

use crate::processor_chain::processor::ProcessPhase;
use crate::processor_chain::registry::{metadata_str, ProcessorRegistry};
use crate::ProcessedMessage;
use closeclaw_common::processor::ContentBlock;

use crate::processor_chain::test_support::{make_normalized, FailingProc, TestProc};

// ── error tolerance ──────────────────────────────────────────────────────────
// A failing processor should not halt the chain; subsequent processors still run.

#[tokio::test]
async fn test_inbound_error_continues_chain() {
    let (ok_proc, ok_counter) = TestProc::inbound("ok_before", 1);
    let fail_proc = FailingProc::inbound("fail_mid");
    let (ok_after, ok_after_counter) = TestProc::inbound("ok_after", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(ok_proc);
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    // ok_before runs and updates ctx, fail_mid fails (ctx unchanged), ok_after runs
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_after_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_inbound_first_processor_fails() {
    let fail_proc = FailingProc::inbound("fail_first");
    let (ok_after, ok_counter) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("original"))
        .await
        .unwrap();

    // fail_first fails (ctx unchanged, original content preserved), ok_after runs
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_inbound_only_processor_fails() {
    let fail_proc = FailingProc::inbound("solo_fail");

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);

    let result = registry
        .process_inbound(make_normalized("raw content"))
        .await
        .unwrap();

    // Single processor fails — ctx unchanged, raw content returned.
    assert_eq!(result.text_content(), Some("raw content"));
}

#[tokio::test]
async fn test_inbound_error_preserves_successful_metadata() {
    let p1 = Arc::new(TestProc {
        name: "p1".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 1,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k1".to_string(), "v1".to_string())),
        suppress: false,
        skip: false,
    });
    let fail_proc = FailingProc::inbound("p_fail");
    let p3 = Arc::new(TestProc {
        name: "p3".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k3".to_string(), "v3".to_string())),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(fail_proc);
    registry.register(p3);

    let result = registry
        .process_inbound(make_normalized("orig"))
        .await
        .unwrap();

    // p1's metadata is preserved, p_fail failed (no metadata), p3's metadata added
    assert_eq!(result.text_content(), Some("p3"));
    assert_eq!(metadata_str(&result.metadata, "k1"), Some("v1"));
    assert_eq!(metadata_str(&result.metadata, "k3"), Some("v3"));
}

#[tokio::test]
async fn test_outbound_error_continues_chain() {
    let (ok_proc, ok_counter) = TestProc::outbound("ok_before", 1);
    let fail_proc = FailingProc::outbound("fail_mid");
    let (ok_after, ok_after_counter) = TestProc::outbound("ok_after", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(ok_proc);
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("llm".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_after_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

// ── outbound error tolerance: detailed tests ──────────────────────────────────

#[tokio::test]
async fn test_outbound_first_processor_fails() {
    let fail_proc = FailingProc::outbound("fail_first");
    let (ok_after, ok_counter) = TestProc::outbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("original".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_outbound_only_processor_fails() {
    let fail_proc = FailingProc::outbound("solo_fail");

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("raw content".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("raw content"));
}

#[tokio::test]
async fn test_outbound_error_preserves_successful_metadata() {
    let p1 = Arc::new(TestProc {
        name: "o1".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 1,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k1".to_string(), "v1".to_string())),
        suppress: false,
        skip: false,
    });
    let fail_proc = FailingProc::outbound("o_fail");
    let p3 = Arc::new(TestProc {
        name: "o3".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k3".to_string(), "v3".to_string())),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(fail_proc);
    registry.register(p3);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("orig".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("o3"));
    assert_eq!(metadata_str(&result.metadata, "k1"), Some("v1"));
    assert_eq!(metadata_str(&result.metadata, "k3"), Some("v3"));
}

// ── log level verification for fail-open handler ─────────────────────────────
// The fail-open handler uses `error!` for `raw_log` processor and
// `warn!` for all other processors. These tests use `tracing_test`
// to capture tracing output per-test and verify the correct log level.

#[tokio::test]
#[traced_test]
async fn test_raw_log_success_no_error_log() {
    let (raw_log, _) = TestProc::inbound("raw_log", 1);
    let (other, _) = TestProc::inbound("session_router", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(raw_log);
    registry.register(other);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("session_router"));
    assert!(
        !logs_contain("ERROR"),
        "no error log expected when raw_log succeeds"
    );
}

#[tokio::test]
#[traced_test]
async fn test_raw_log_failure_produces_error_log() {
    let fail_raw = FailingProc::inbound("raw_log");
    let (ok_after, _) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_raw);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("ok_after"));
    assert!(logs_contain("ERROR"), "expected ERROR for raw_log failure");
    assert!(!logs_contain("WARN"), "raw_log should use ERROR, not WARN");
}

#[tokio::test]
#[traced_test]
async fn test_non_raw_log_failure_produces_warn_log() {
    let fail_router = FailingProc::inbound("session_router");
    let (ok_after, _) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_router);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("ok_after"));
    assert!(
        logs_contain("WARN"),
        "expected WARN for session_router failure"
    );
    assert!(
        !logs_contain("ERROR"),
        "non raw_log should use WARN, not ERROR"
    );
}

#[tokio::test]
#[traced_test]
async fn test_fail_open_continues_chain_regardless_of_processor() {
    // raw_log fails (error!), session_router succeeds,
    // content_normalizer fails (warn!), last processor runs.
    let fail_raw = FailingProc::inbound("raw_log");
    let (ok_mid, ok_counter) = TestProc::inbound("session_router", 2);
    let fail_norm = FailingProc::inbound("content_normalizer");
    let (ok_end, ok_end_counter) = TestProc::inbound("final_proc", 4);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_raw);
    registry.register(ok_mid);
    registry.register(fail_norm);
    registry.register(ok_end);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    // All non-failing processors ran.
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_end_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("final_proc"));

    // raw_log failure → ERROR
    assert!(logs_contain("ERROR"), "expected ERROR for raw_log failure");
    // content_normalizer failure → WARN
    assert!(
        logs_contain("WARN"),
        "expected WARN for content_normalizer failure"
    );
}
