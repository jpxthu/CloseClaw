use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use tokio::time::sleep;

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_call(
    id: &str,
    tool_name: &str,
    is_concurrency_safe: bool,
    file_path: Option<&str>,
) -> PendingToolCall {
    PendingToolCall {
        id: id.into(),
        tool_name: tool_name.into(),
        args: serde_json::json!({}),
        file_path: file_path.map(PathBuf::from),
        is_concurrency_safe,
    }
}

/// A test executor that records execution order and returns a marker result.
struct RecordingExecutor {
    log: Arc<std::sync::Mutex<Vec<String>>>,
}

impl RecordingExecutor {
    fn new() -> (Self, Arc<std::sync::Mutex<Vec<String>>>) {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        (Self { log: log.clone() }, log)
    }
}

#[async_trait]
impl ToolExecutor for RecordingExecutor {
    async fn execute(&self, call: &PendingToolCall) -> closeclaw_common::tool_trait::ToolResult {
        self.log.lock().unwrap().push(call.id.clone());
        closeclaw_common::tool_trait::ToolResult {
            data: serde_json::json!({ "id": call.id }),
            new_messages: vec![],
            context_modifier: None,
        }
    }
}

// ---------------------------------------------------------------------------
// classify tests
// ---------------------------------------------------------------------------

#[test]
fn test_classify_concurrent_safe_returns_parallel() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let call = make_call("1", "Read", true, Some("/a/file.txt"));
    assert_eq!(dispatcher.classify(&call), DispatchGroup::Parallel);
}

#[test]
fn test_classify_unsafe_with_file_returns_mutex() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let call = make_call("1", "Edit", false, Some("/a/file.txt"));
    assert_eq!(dispatcher.classify(&call), DispatchGroup::MutexByFile);
}

#[test]
fn test_classify_unsafe_without_file_returns_serial() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let call = make_call("1", "Bash", false, None);
    assert_eq!(dispatcher.classify(&call), DispatchGroup::Serial);
}

#[test]
fn test_classify_parallel_disabled_returns_serial() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), false);
    let call = make_call("1", "Read", true, None);
    assert_eq!(dispatcher.classify(&call), DispatchGroup::Serial);
}

// ---------------------------------------------------------------------------
// dispatch_all tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_calls() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, _log) = RecordingExecutor::new();
    let results = dispatcher.dispatch_all(vec![], &exec).await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_parallel_execution() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, log) = RecordingExecutor::new();
    let calls = vec![
        make_call("r1", "Read", true, Some("/a.txt")),
        make_call("r2", "Read", true, Some("/b.txt")),
        make_call("r3", "Read", true, Some("/c.txt")),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 3);

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 3);
    assert!(log.contains(&"r1".to_string()));
    assert!(log.contains(&"r2".to_string()));
    assert!(log.contains(&"r3".to_string()));
}

#[tokio::test]
async fn test_parallel_truly_concurrent() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);

    struct ConcurrentExecutor;
    #[async_trait]
    impl ToolExecutor for ConcurrentExecutor {
        async fn execute(
            &self,
            _call: &PendingToolCall,
        ) -> closeclaw_common::tool_trait::ToolResult {
            sleep(Duration::from_millis(10)).await;
            closeclaw_common::tool_trait::ToolResult {
                data: serde_json::json!(null),
                new_messages: vec![],
                context_modifier: None,
            }
        }
    }

    let calls = vec![
        make_call("p1", "Read", true, None),
        make_call("p2", "Read", true, None),
    ];

    let start = std::time::Instant::now();
    let _results = dispatcher.dispatch_all(calls, &ConcurrentExecutor).await;
    let elapsed = start.elapsed();

    // Two futures each sleeping ~10ms, polled concurrently via join_all.
    // Should finish in ~10ms, not ~20ms.
    assert!(
        elapsed < Duration::from_millis(25),
        "Expected concurrent execution, took {elapsed:?}"
    );
}

#[tokio::test]
async fn test_same_file_mutex_serial() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);

    let counter = Arc::new(AtomicUsize::new(0));

    struct SerialDetectingExecutor {
        counter: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl ToolExecutor for SerialDetectingExecutor {
        async fn execute(
            &self,
            _call: &PendingToolCall,
        ) -> closeclaw_common::tool_trait::ToolResult {
            let before = self.counter.load(Ordering::SeqCst);
            tokio::task::yield_now().await;
            let after = self.counter.fetch_add(1, Ordering::SeqCst);
            // If serialized, before always equals after
            assert_eq!(
                before, after,
                "Concurrent execution detected for same-file calls"
            );
            closeclaw_common::tool_trait::ToolResult {
                data: serde_json::json!(null),
                new_messages: vec![],
                context_modifier: None,
            }
        }
    }

    let calls = vec![
        make_call("m1", "Edit", false, Some("/same.txt")),
        make_call("m2", "Edit", false, Some("/same.txt")),
    ];
    let exec = SerialDetectingExecutor {
        counter: counter.clone(),
    };
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_different_files_parallel() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);

    struct ParallelDetectingExecutor;
    #[async_trait]
    impl ToolExecutor for ParallelDetectingExecutor {
        async fn execute(
            &self,
            _call: &PendingToolCall,
        ) -> closeclaw_common::tool_trait::ToolResult {
            sleep(Duration::from_millis(10)).await;
            closeclaw_common::tool_trait::ToolResult {
                data: serde_json::json!(null),
                new_messages: vec![],
                context_modifier: None,
            }
        }
    }

    let calls = vec![
        make_call("d1", "Edit", false, Some("/file_a.txt")),
        make_call("d2", "Edit", false, Some("/file_b.txt")),
    ];

    let start = std::time::Instant::now();
    let results = dispatcher
        .dispatch_all(calls, &ParallelDetectingExecutor)
        .await;
    let elapsed = start.elapsed();
    assert_eq!(results.len(), 2);

    assert!(
        elapsed < Duration::from_millis(35),
        "Expected parallel execution for different files, took {elapsed:?}"
    );
}

#[tokio::test]
async fn test_serial_fallback_when_disabled() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), false);
    let (exec, log) = RecordingExecutor::new();
    let calls = vec![
        make_call("s1", "Read", true, None),
        make_call("s2", "Edit", false, Some("/a.txt")),
        make_call("s3", "Read", true, Some("/b.txt")),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 3);

    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["s1", "s2", "s3"]);
}

#[tokio::test]
async fn test_read_edit_same_file_ordering() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, log) = RecordingExecutor::new();

    let calls = vec![
        make_call("e", "Edit", false, Some("/shared.txt")),
        make_call("r", "Read", true, Some("/shared.txt")),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 2);

    let log = log.lock().unwrap();
    let r_pos = log.iter().position(|id| id == "r").unwrap();
    let e_pos = log.iter().position(|id| id == "e").unwrap();
    assert!(
        r_pos < e_pos,
        "Read ({r_pos}) should execute before Edit ({e_pos})"
    );
}

#[tokio::test]
async fn test_result_order_matches_input() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, _log) = RecordingExecutor::new();

    let calls = vec![
        make_call("a", "Read", true, None),
        make_call("b", "Edit", false, Some("/f1.txt")),
        make_call("c", "Read", true, None),
        make_call("d", "Edit", false, Some("/f2.txt")),
    ];
    let expected_ids: Vec<String> = calls.iter().map(|c| c.id.clone()).collect();
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 4);

    for (i, result) in results.iter().enumerate() {
        let actual_id = result.data.get("id").and_then(|v| v.as_str()).unwrap();
        assert_eq!(actual_id, expected_ids[i]);
    }
}

#[tokio::test]
async fn test_mixed_groups_correct_classification() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, log) = RecordingExecutor::new();

    let calls = vec![
        make_call("p1", "Read", true, Some("/x.txt")),
        make_call("p2", "Read", true, Some("/y.txt")),
        make_call("m1", "Edit", false, Some("/x.txt")),
        make_call("m2", "Edit", false, Some("/z.txt")),
        make_call("s1", "Bash", false, None),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 5);

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 5);
}

#[tokio::test]
async fn test_single_call() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, log) = RecordingExecutor::new();
    let calls = vec![make_call("only", "Read", true, None)];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 1);
    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["only"]);
}

// ---------------------------------------------------------------------------
// should_fallback_to_serial
// ---------------------------------------------------------------------------

#[test]
fn test_fallback_serial_when_parallel_disabled() {
    let d = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), false);
    assert!(d.should_fallback_to_serial(true));
    assert!(d.should_fallback_to_serial(false));
}

#[test]
fn test_fallback_serial_when_provider_unsupported() {
    let d = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    assert!(!d.should_fallback_to_serial(true));
    assert!(d.should_fallback_to_serial(false));
}

#[tokio::test]
async fn test_fallback_all_serial_via_dispatch() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), false);
    let (exec, log) = RecordingExecutor::new();
    let calls = vec![
        make_call("x", "Read", true, None),
        make_call("y", "Edit", false, Some("/a.txt")),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 2);

    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["x", "y"]);
}

// ---------------------------------------------------------------------------
// Multi-file Read+Edit race test
// ---------------------------------------------------------------------------

/// Verifies that when multiple Read calls share a file with multiple Edit
/// calls, all Read calls complete before any Edit on that file starts.
#[tokio::test]
async fn test_multi_file_read_edit_race() {
    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let (exec, log) = RecordingExecutor::new();

    // Two Reads and two Edits on the same file, interleaved in input order.
    let calls = vec![
        make_call("edit1", "Edit", false, Some("/race.txt")),
        make_call("read1", "Read", true, Some("/race.txt")),
        make_call("edit2", "Edit", false, Some("/race.txt")),
        make_call("read2", "Read", true, Some("/race.txt")),
    ];
    let results = dispatcher.dispatch_all(calls, &exec).await;
    assert_eq!(results.len(), 4);

    let log = log.lock().unwrap();
    // Both Reads should appear before both Edits in the execution log.
    let r1_pos = log.iter().position(|id| id == "read1").unwrap();
    let r2_pos = log.iter().position(|id| id == "read2").unwrap();
    let e1_pos = log.iter().position(|id| id == "edit1").unwrap();
    let e2_pos = log.iter().position(|id| id == "edit2").unwrap();
    assert!(
        r1_pos < e1_pos && r1_pos < e2_pos,
        "read1 ({r1_pos}) should execute before edit1 ({e1_pos}) and edit2 ({e2_pos})"
    );
    assert!(
        r2_pos < e1_pos && r2_pos < e2_pos,
        "read2 ({r2_pos}) should execute before edit1 ({e1_pos}) and edit2 ({e2_pos})"
    );
}

// ---------------------------------------------------------------------------
// FileMutexMap cleanup verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dispatch_cleanup_no_residual_entries() {
    let mutex_map = Arc::new(FileMutexMap::new());
    let dispatcher = ToolCallDispatcher::new(mutex_map.clone(), true);
    let (exec, _log) = RecordingExecutor::new();

    let calls = vec![
        make_call("e1", "Edit", false, Some("/file_a.txt")),
        make_call("e2", "Edit", false, Some("/file_a.txt")),
        make_call("e3", "Edit", false, Some("/file_b.txt")),
    ];
    let _results = dispatcher.dispatch_all(calls, &exec).await;

    // After dispatch, all entries should have been cleaned up.
    assert_eq!(
        mutex_map.len(),
        0,
        "Expected FileMutexMap to be empty after dispatch, got {} entries",
        mutex_map.len()
    );
}

// ---------------------------------------------------------------------------
// extract_file_path tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_file_path_from_path_key() {
    let args = serde_json::json!({"path": "/a/b.txt"});
    assert_eq!(extract_file_path(&args), Some(PathBuf::from("/a/b.txt")));
}

#[test]
fn test_extract_file_path_from_file_path_key() {
    let args = serde_json::json!({"file_path": "/c/d.txt"});
    assert_eq!(extract_file_path(&args), Some(PathBuf::from("/c/d.txt")));
}

#[test]
fn test_extract_file_path_prefers_path_key() {
    let args = serde_json::json!({"path": "/first.txt", "file_path": "/second.txt"});
    assert_eq!(extract_file_path(&args), Some(PathBuf::from("/first.txt")));
}

#[test]
fn test_extract_file_path_none_when_absent() {
    let args = serde_json::json!({"command": "echo hi"});
    assert_eq!(extract_file_path(&args), None);
}

#[test]
fn test_extract_file_path_none_for_empty_args() {
    let args = serde_json::json!({});
    assert_eq!(extract_file_path(&args), None);
}

// ---------------------------------------------------------------------------
// build_pending_call tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_pending_call_looks_up_flags() {
    let registry = crate::ToolRegistryImpl::new();
    // Register a concurrency-safe read-only tool.
    use crate::Tool;
    struct DummyReadTool;
    #[async_trait]
    impl Tool for DummyReadTool {
        fn name(&self) -> &str {
            "DummyRead"
        }
        fn group(&self) -> &str {
            "test"
        }
        fn summary(&self) -> String {
            "dummy".into()
        }
        fn detail(&self) -> String {
            "dummy".into()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}})
        }
        fn flags(&self) -> crate::ToolFlags {
            crate::ToolFlags {
                is_concurrency_safe: true,
                is_read_only: true,
                ..Default::default()
            }
        }
    }
    registry.register(DummyReadTool).await.unwrap();

    let args = serde_json::json!({"path": "/test/file.txt"});
    let pending = build_pending_call("call-1".into(), "DummyRead", args.clone(), &registry).await;

    assert_eq!(pending.id, "call-1");
    assert_eq!(pending.tool_name, "DummyRead");
    assert_eq!(pending.args, args);
    assert_eq!(pending.file_path, Some(PathBuf::from("/test/file.txt")));
    assert!(pending.is_concurrency_safe);
}

#[tokio::test]
async fn test_build_pending_call_no_file_path() {
    let registry = crate::ToolRegistryImpl::new();
    use crate::Tool;
    struct NoPathTool;
    #[async_trait]
    impl Tool for NoPathTool {
        fn name(&self) -> &str {
            "NoPath"
        }
        fn group(&self) -> &str {
            "test"
        }
        fn summary(&self) -> String {
            "no-path".into()
        }
        fn detail(&self) -> String {
            "no-path".into()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}})
        }
        fn flags(&self) -> crate::ToolFlags {
            crate::ToolFlags {
                is_concurrency_safe: false,
                ..Default::default()
            }
        }
    }
    registry.register(NoPathTool).await.unwrap();

    let args = serde_json::json!({"command": "echo hi"});
    let pending = build_pending_call("call-2".into(), "NoPath", args, &registry).await;

    assert_eq!(pending.file_path, None);
    assert!(!pending.is_concurrency_safe);
}

// ---------------------------------------------------------------------------
// ToolRegistryExecutor tests
// ---------------------------------------------------------------------------

use closeclaw_common::tool_trait::ToolContext;

/// A simple tool that echoes its args back.
struct EchoTool;

#[async_trait]
impl crate::Tool for EchoTool {
    fn name(&self) -> &str {
        "Echo"
    }
    fn group(&self) -> &str {
        "test"
    }
    fn summary(&self) -> String {
        "echo".into()
    }
    fn detail(&self) -> String {
        "echo".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"msg": {"type": "string"}}})
    }
    async fn call(
        &self,
        args: Value,
        _ctx: &ToolContext,
    ) -> Result<closeclaw_common::tool_trait::ToolResult, closeclaw_common::tool_trait::ToolCallError>
    {
        Ok(closeclaw_common::tool_trait::ToolResult {
            data: args,
            new_messages: vec![],
            context_modifier: None,
        })
    }
    fn flags(&self) -> crate::ToolFlags {
        crate::ToolFlags {
            is_concurrency_safe: true,
            ..Default::default()
        }
    }
}

fn make_base_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

#[tokio::test]
async fn test_executor_looks_up_and_calls_tool() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(EchoTool).await.unwrap();

    let executor = ToolRegistryExecutor::new(registry, make_base_ctx());
    let call = PendingToolCall {
        id: "exec-1".into(),
        tool_name: "Echo".into(),
        args: serde_json::json!({"msg": "hello"}),
        file_path: None,
        is_concurrency_safe: true,
    };

    let result = executor.execute(&call).await;
    assert_eq!(result.data, serde_json::json!({"msg": "hello"}));
}

#[tokio::test]
async fn test_executor_sets_call_id() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());

    // A tool that captures call_id from ToolContext.
    struct CallIdCaptureTool;
    #[async_trait]
    impl crate::Tool for CallIdCaptureTool {
        fn name(&self) -> &str {
            "Capture"
        }
        fn group(&self) -> &str {
            "test"
        }
        fn summary(&self) -> String {
            "capture".into()
        }
        fn detail(&self) -> String {
            "capture".into()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn call(
            &self,
            _args: Value,
            ctx: &ToolContext,
        ) -> Result<
            closeclaw_common::tool_trait::ToolResult,
            closeclaw_common::tool_trait::ToolCallError,
        > {
            Ok(closeclaw_common::tool_trait::ToolResult {
                data: serde_json::json!({"call_id": ctx.call_id}),
                new_messages: vec![],
                context_modifier: None,
            })
        }
        fn flags(&self) -> crate::ToolFlags {
            crate::ToolFlags::default()
        }
    }
    registry.register(CallIdCaptureTool).await.unwrap();

    let executor = ToolRegistryExecutor::new(registry, make_base_ctx());
    let call = PendingToolCall {
        id: "my-call-id".into(),
        tool_name: "Capture".into(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: false,
    };

    let result = executor.execute(&call).await;
    assert_eq!(
        result.data["call_id"],
        serde_json::json!("my-call-id"),
        "executor should set call_id from PendingToolCall.id"
    );
}

/// A tool that always fails.
struct FailTool;

#[async_trait]
impl crate::Tool for FailTool {
    fn name(&self) -> &str {
        "Fail"
    }
    fn group(&self) -> &str {
        "test"
    }
    fn summary(&self) -> String {
        "fail".into()
    }
    fn detail(&self) -> String {
        "fail".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn call(
        &self,
        _args: Value,
        _ctx: &ToolContext,
    ) -> Result<closeclaw_common::tool_trait::ToolResult, closeclaw_common::tool_trait::ToolCallError>
    {
        Err(closeclaw_common::tool_trait::ToolCallError::NotImplemented)
    }
    fn flags(&self) -> crate::ToolFlags {
        crate::ToolFlags::default()
    }
}

#[tokio::test]
async fn test_executor_tool_error_becomes_result() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FailTool).await.unwrap();

    let executor = ToolRegistryExecutor::new(registry, make_base_ctx());
    let call = PendingToolCall {
        id: "fail-1".into(),
        tool_name: "Fail".into(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: false,
    };

    let result = executor.execute(&call).await;
    // ToolCallError is converted to a ToolResult with error info.
    assert!(
        result.data.get("error").is_some(),
        "failed tool should produce error in result data"
    );
}

#[tokio::test]
async fn test_executor_nonexistent_tool_panics() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    let executor = ToolRegistryExecutor::new(registry, make_base_ctx());
    let call = PendingToolCall {
        id: "x".into(),
        tool_name: "Nonexistent".into(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: false,
    };

    let result = std::panic::AssertUnwindSafe(executor.execute(&call))
        .catch_unwind()
        .await;
    assert!(result.is_err(), "should panic for unknown tool");
}

// ---------------------------------------------------------------------------
// End-to-end: dispatcher + ToolRegistryExecutor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_end_to_end_dispatch_with_real_executor() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(EchoTool).await.unwrap();

    let dispatcher = ToolCallDispatcher::new(Arc::new(FileMutexMap::new()), true);
    let executor = ToolRegistryExecutor::new(registry, make_base_ctx());

    let calls = vec![
        PendingToolCall {
            id: "e2e-1".into(),
            tool_name: "Echo".into(),
            args: serde_json::json!({"msg": "hi"}),
            file_path: None,
            is_concurrency_safe: true,
        },
        PendingToolCall {
            id: "e2e-2".into(),
            tool_name: "Echo".into(),
            args: serde_json::json!({"msg": "bye"}),
            file_path: None,
            is_concurrency_safe: true,
        },
    ];

    let results = dispatcher.dispatch_all(calls, &executor).await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].data, serde_json::json!({"msg": "hi"}));
    assert_eq!(results[1].data, serde_json::json!({"msg": "bye"}));
}
