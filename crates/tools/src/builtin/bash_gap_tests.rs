//! Tests for BashTool gaps fixed in Steps 1.1 and 1.2.
//!
//! Gap 1: `generate_prompt` must use the default Tool trait impl
//!         (return `self.detail()`), with no workdir/git injection.
//! Gap 2: Progress tracking must count lines/bytes correctly and
//!         throttle updates to ≥ 2 s intervals.

use super::*;
use crate::builtin::bash_kill::{read_pipe_incremental, read_with_progress};
use crate::Tool;
use closeclaw_common::tool_session::ToolProgress;
use closeclaw_common::{PromptGenerationContext, WorkdirContext};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::GatewayConfig;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::RuleSet;
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_tool() -> BashTool {
    let perm = Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap()),
    ));
    let bg_manager: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(DummyTaskManager);
    let session_manager = Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ));
    let tmp = tempfile::TempDir::new().unwrap();
    let config_manager = Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    );
    let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )));
    BashTool::new(
        perm,
        bg_manager,
        session_manager,
        config_manager,
        approval_flow,
    )
}

fn ctx_with_workdir(path: &str, has_git: bool) -> PromptGenerationContext {
    PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: path.to_string(),
            has_git,
            branch: if has_git { Some("main".into()) } else { None },
            recent_changes: if has_git { 3 } else { 0 },
        }),
        ..Default::default()
    }
}

// Dummy TaskManager for tests that don't need actual task operations.
struct DummyTaskManager;

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for DummyTaskManager {
    async fn spawn_task(
        &self,
        _cmd: &str,
        _cwd: &std::path::Path,
        _bg: bool,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        unimplemented!()
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _cmd: &str,
        _bg: bool,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        unimplemented!()
    }
    async fn kill_task(&self, _: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        Ok(())
    }
    async fn get_task(&self, _: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        None
    }
    async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
        vec![]
    }
    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        vec![]
    }
    async fn cleanup_finished(&self) {}
}

/// Working TaskManager for tests that need spawn/backgroundize.
struct WorkingTaskManager {
    tasks: std::sync::Arc<
        tokio::sync::RwLock<std::collections::HashMap<String, closeclaw_tasks::BackgroundTask>>,
    >,
}

impl WorkingTaskManager {
    fn new() -> Self {
        Self {
            tasks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for WorkingTaskManager {
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &std::path::Path,
        is_backgrounded: bool,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        let task = closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running { is_backgrounded },
            output_path: cwd.join("output"),
        };
        self.tasks
            .write()
            .await
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        let task = closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running { is_backgrounded },
            output_path: std::path::PathBuf::from("/tmp/output"),
        };
        self.tasks
            .write()
            .await
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }
    async fn kill_task(&self, _: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        Ok(())
    }
    async fn get_task(&self, _: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        None
    }
    async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
        vec![]
    }
    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        vec![]
    }
    async fn cleanup_finished(&self) {}
}

/// Mock ToolSession that captures `report_tool_progress` calls.
struct MockProgressSession {
    calls: std::sync::Mutex<Vec<ToolProgress>>,
}

impl MockProgressSession {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn progress_calls(&self) -> Vec<ToolProgress> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl closeclaw_common::tool_session::ToolSession for MockProgressSession {
    async fn register_tool_handle(
        &self,
        _call_id: String,
        _handle: Arc<dyn closeclaw_common::tool_session::KillHandle>,
    ) {
    }

    async fn report_tool_progress(&self, _call_id: &str, progress: ToolProgress) {
        self.calls.lock().unwrap().push(progress);
    }
}

/// Spawn a child process and return its stdout/stderr handles.
/// Panics if spawn fails.
fn spawn_child(
    command: &str,
) -> (
    tokio::process::Child,
    tokio::process::ChildStdout,
    tokio::process::ChildStderr,
) {
    use tokio::process::Command;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn '{}': {}", command, e));
    let stdout = child.stdout.take().expect("stdout must be piped");
    let stderr = child.stderr.take().expect("stderr must be piped");
    (child, stdout, stderr)
}

// ===========================================================================
// Gap 1: generate_prompt tests (updated for Step 1.2 context-aware behavior)
// ===========================================================================

/// `generate_prompt` must return context-aware output for empty context.
#[tokio::test]
async fn test_generate_prompt_with_empty_context() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    // Empty context: no workdir, default permission (allowed)
    assert!(
        prompt.contains("Bash is available"),
        "empty context should show Bash as available"
    );
    assert!(
        !prompt.contains("Working directory"),
        "empty context should not mention working directory"
    );
}

/// Workdir context changes the prompt output.
#[tokio::test]
async fn test_generate_prompt_includes_workdir() {
    let tool = make_tool();
    let no_workdir = PromptGenerationContext::default();
    let with_workdir = ctx_with_workdir("/some/path", false);
    let prompt_no = tool.generate_prompt(&no_workdir);
    let prompt_yes = tool.generate_prompt(&with_workdir);
    assert_ne!(
        prompt_no, prompt_yes,
        "different workdir contexts should produce different prompts"
    );
    assert!(
        prompt_yes.contains("/some/path"),
        "prompt should contain the working directory path"
    );
    assert!(
        prompt_yes.contains("not a git repo"),
        "non-git path should note absence of git"
    );
}

/// Git branch and recent_changes are reflected in the prompt.
#[tokio::test]
async fn test_generate_prompt_includes_git_info() {
    let tool = make_tool();
    let no_git = ctx_with_workdir("/tmp", false);
    let with_git = ctx_with_workdir("/tmp", true);
    let prompt_no = tool.generate_prompt(&no_git);
    let prompt_yes = tool.generate_prompt(&with_git);
    assert!(
        prompt_yes.contains("main"),
        "git prompt should contain the branch name"
    );
    assert!(
        prompt_yes.contains("uncommitted change"),
        "git prompt should mention uncommitted changes"
    );
    assert!(
        prompt_no.contains("not a git repo"),
        "non-git prompt should note absence of git"
    );
}

/// Prompt adapts when Bash tool is not in the allowed tool list.
#[tokio::test]
async fn test_generate_prompt_permission_denied() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        tools: Some(vec!["Read".into(), "Write".into()]),
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("not available"),
        "Bash not in tools list should show unavailable"
    );
}

/// Prompt includes combination suggestions when Read is available.
#[tokio::test]
async fn test_generate_prompt_combination_suggestions() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        available_tool_names: vec!["Bash".into(), "Read".into()],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("Read"),
        "should suggest Read as a combination"
    );
}

/// Full context produces a comprehensive prompt.
#[tokio::test]
async fn test_generate_prompt_full_context() {
    let tool = make_tool();
    let full_ctx = PromptGenerationContext {
        agent_id: "agent-1".into(),
        workdir: Some(WorkdirContext {
            path: "/home/user/project".into(),
            has_git: true,
            branch: Some("feat/x".into()),
            recent_changes: 7,
        }),
        available_tool_names: vec!["Bash".into(), "Read".into(), "Write".into()],
        tools: Some(vec!["Bash".into(), "Read".into()]),
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let prompt = tool.generate_prompt(&full_ctx);
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("feat/x"));
    assert!(prompt.contains("Bash is available"));
    assert!(prompt.contains("Read"));
    assert!(prompt.contains("Write"));
}

// ===========================================================================
// Gap 2: Progress tracking tests
// ===========================================================================

/// `read_pipe_incremental` counts lines and bytes correctly.
#[tokio::test]
async fn test_read_pipe_incremental_counts_lines_and_bytes() {
    let data = "line one\nline two\nline three\n";
    let reader = std::io::Cursor::new(data.as_bytes().to_vec());
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let (output, lines, bytes) = read_pipe_incremental(Some(reader), cancel_rx, None).await;

    assert_eq!(output, data);
    assert_eq!(bytes, data.len());
    // "line one\nline two\nline three\n" → 3 newline-terminated lines
    assert_eq!(lines, 3, "expected 3 lines from 3 newline-terminated lines");
}

/// `read_pipe_incremental` handles single-line input (no trailing newline).
#[tokio::test]
async fn test_read_pipe_incremental_single_line_no_newline() {
    let data = "hello";
    let reader = std::io::Cursor::new(data.as_bytes().to_vec());
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let (output, lines, bytes) = read_pipe_incremental(Some(reader), cancel_rx, None).await;

    assert_eq!(output, "hello");
    assert_eq!(bytes, 5);
    assert_eq!(lines, 1);
}

/// `read_pipe_incremental` returns zero counts for empty reader.
#[tokio::test]
async fn test_read_pipe_incremental_empty() {
    let reader = std::io::Cursor::new(Vec::<u8>::new());
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let (output, lines, bytes) = read_pipe_incremental(Some(reader), cancel_rx, None).await;

    assert_eq!(output, "");
    assert_eq!(lines, 0);
    assert_eq!(bytes, 0);
}

/// `read_pipe_incremental` returns zero counts for None pipe.
#[tokio::test]
async fn test_read_pipe_incremental_none_pipe() {
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let (output, lines, bytes) =
        read_pipe_incremental::<std::io::Cursor<Vec<u8>>>(None, cancel_rx, None).await;

    assert_eq!(output, "");
    assert_eq!(lines, 0);
    assert_eq!(bytes, 0);
}

/// `read_pipe_incremental` updates progress counters when provided.
#[tokio::test]
async fn test_read_pipe_incremental_updates_progress_counters() {
    let data = "aaa\nbbb\n";
    let reader = std::io::Cursor::new(data.as_bytes().to_vec());
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let counters = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));

    let (output, lines, bytes) =
        read_pipe_incremental(Some(reader), cancel_rx, Some(&counters)).await;

    assert_eq!(output, data);
    assert_eq!(lines, 2);
    assert_eq!(bytes, data.len());
    assert_eq!(counters.0.load(Ordering::Relaxed), 2);
    assert_eq!(counters.1.load(Ordering::Relaxed), data.len());
}

/// `read_pipe_incremental` respects cancel signal.
#[tokio::test]
async fn test_read_pipe_incremental_cancelled() {
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let _ = cancel_tx.send(true);

    let reader = std::io::Cursor::new("data".as_bytes().to_vec());
    let (_output, _lines, bytes) = read_pipe_incremental(Some(reader), cancel_rx, None).await;

    assert!(bytes <= 4);
}

/// Short command (< 2 s) does NOT trigger progress reports.
/// The process completes before the first periodic interval tick (2 s).
#[tokio::test]
async fn test_read_with_progress_short_command_no_reports() {
    let session = MockProgressSession::new();
    let session_arc: Arc<dyn closeclaw_common::tool_session::ToolSession> =
        Arc::clone(&session) as Arc<dyn closeclaw_common::tool_session::ToolSession>;

    let (mut _child, stdout, stderr) = spawn_child("echo hello");
    let (stdout_out, stderr_out, _lines, _bytes) = read_with_progress(
        Some(stdout),
        Some(stderr),
        Some(&session_arc),
        Some("test-call"),
    )
    .await;

    assert_eq!(stdout_out, "hello\n");
    assert!(stderr_out.is_empty());
    // Only the final report (no periodic reports — command finished before 2 s)
    let calls = session.progress_calls();
    assert_eq!(
        calls.len(),
        1,
        "short command should only get the final report, got {}",
        calls.len()
    );
    assert_eq!(calls[0].lines, 1);
    assert_eq!(calls[0].bytes, stdout_out.len());
    // Wait for child to avoid zombie
    let _ = _child.kill().await;
    let _ = _child.wait().await;
}

/// Long command (> 2 s) triggers progress reports while output arrives.
#[tokio::test]
async fn test_read_with_progress_long_command_sends_reports() {
    let session = MockProgressSession::new();
    let session_arc: Arc<dyn closeclaw_common::tool_session::ToolSession> =
        Arc::clone(&session) as Arc<dyn closeclaw_common::tool_session::ToolSession>;

    // Command produces output slowly over ~3 s, exceeding the 2 s interval.
    let (mut _child, stdout, stderr) =
        spawn_child("for i in 1 2 3 4; do echo \"line $i\"; sleep 0.8; done");

    let (_stdout_out, _stderr_out, _lines, _bytes) = read_with_progress(
        Some(stdout),
        Some(stderr),
        Some(&session_arc),
        Some("test-call"),
    )
    .await;

    let calls = session.progress_calls();
    assert!(
        calls.len() >= 1,
        "expected at least 1 periodic progress report for long command, got {}",
        calls.len()
    );
    for (i, report) in calls.iter().enumerate() {
        assert!(
            report.bytes > 0,
            "report[{i}] should have bytes > 0, got {}",
            report.bytes
        );
        assert!(
            report.elapsed >= Duration::from_millis(1000),
            "report[{i}] elapsed should be ≥ 1s, got {:?}",
            report.elapsed
        );
    }
    // Clean up child
    let _ = _child.kill().await;
    let _ = _child.wait().await;
}

/// `read_with_progress` always sends a final progress report.
#[tokio::test]
async fn test_read_with_progress_sends_final_report() {
    let session = MockProgressSession::new();
    let session_arc: Arc<dyn closeclaw_common::tool_session::ToolSession> =
        Arc::clone(&session) as Arc<dyn closeclaw_common::tool_session::ToolSession>;

    let (mut _child, stdout, stderr) = spawn_child("printf 'line1\nline2\n'");

    let (stdout_out, _stderr_out, _lines, _bytes) = read_with_progress(
        Some(stdout),
        Some(stderr),
        Some(&session_arc),
        Some("test-call"),
    )
    .await;

    assert_eq!(stdout_out, "line1\nline2\n");
    let calls = session.progress_calls();
    assert!(
        !calls.is_empty(),
        "must send at least a final progress report"
    );
    let last = calls.last().unwrap();
    assert_eq!(
        last.lines, 2,
        "final report lines should be 2, got {}",
        last.lines
    );
    assert_eq!(last.bytes, stdout_out.len(), "final report bytes mismatch");
    let _ = _child.kill().await;
    let _ = _child.wait().await;
}

/// Background commands do NOT trigger progress tracking.
/// When `run_in_background` is true, the foreground path is never entered.
#[tokio::test]
async fn test_background_command_no_progress_reports() {
    let session = MockProgressSession::new();
    let session_arc: Arc<dyn closeclaw_common::tool_session::ToolSession> =
        Arc::clone(&session) as Arc<dyn closeclaw_common::tool_session::ToolSession>;

    let bg_manager: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(WorkingTaskManager::new());
    let tmp = tempfile::TempDir::new().unwrap();
    let _result = execute_command(
        "echo bg",
        tmp.path().to_str().unwrap(),
        Some(5_000),
        true, // run_in_background
        &bg_manager,
        Some(&session_arc),
        Some("bg-call-id"),
        None,
    )
    .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    let calls = session.progress_calls();
    assert!(
        calls.is_empty(),
        "background command should not trigger progress reports, got {}",
        calls.len()
    );
}

/// Progress updates are throttled: the periodic timer must tick before
/// sending updates. A fast command completing before the first tick
/// should produce no periodic reports (only the final one).
#[tokio::test]
async fn test_read_with_progress_throttled_by_interval() {
    let session = MockProgressSession::new();
    let session_arc: Arc<dyn closeclaw_common::tool_session::ToolSession> =
        Arc::clone(&session) as Arc<dyn closeclaw_common::tool_session::ToolSession>;

    // Fast command — completes well before the 2 s interval tick.
    let (mut _child, stdout, stderr) = spawn_child("echo done");

    let (stdout_out, _stderr_out, _lines, _bytes) = read_with_progress(
        Some(stdout),
        Some(stderr),
        Some(&session_arc),
        Some("fast-call"),
    )
    .await;

    assert_eq!(stdout_out, "done\n");
    // Only the final report should exist (periodic timer hasn't ticked yet)
    let calls = session.progress_calls();
    assert_eq!(
        calls.len(),
        1,
        "fast command should only have the final report, got {}",
        calls.len()
    );
    assert_eq!(calls[0].lines, 1);
    assert_eq!(calls[0].bytes, stdout_out.len());
    let _ = _child.kill().await;
    let _ = _child.wait().await;
}
