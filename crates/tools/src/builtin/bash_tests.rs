//! Unit tests for BashTool (extracted from inline tests).
//!
//! These tests are compiled via `#[path = "bash_tests.rs"]` inside
//! `bash.rs`'s `#[cfg(test)] mod tests`, which grants access to
//! private items in the parent module.

use super::*;
use crate::builtin::bash_kill::{persist_output, process_output, MAX_OUTPUT_CHARS};
use closeclaw_permission::approval_flow::HeartbeatApprovalMode;
use serde_json::json;
use tempfile::TempDir;

fn test_permission_engine() -> Arc<PermissionEngine> {
    use closeclaw_permission::rules::RuleSetBuilder;
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn test_bg_manager() -> Arc<dyn closeclaw_tasks::TaskManager> {
    Arc::new(BackgroundTaskManager::new())
}

struct BackgroundTaskManager {
    tasks: std::sync::Arc<
        tokio::sync::RwLock<std::collections::HashMap<String, closeclaw_tasks::BackgroundTask>>,
    >,
}

impl BackgroundTaskManager {
    fn new() -> Self {
        Self {
            tasks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }
    async fn get_task(&self, id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        self.tasks.read().await.get(id).cloned()
    }
    async fn is_running(&self, id: &str) -> bool {
        self.tasks
            .read()
            .await
            .get(id)
            .map(|t| t.state == closeclaw_tasks::TaskState::Running)
            .unwrap_or(false)
    }
    async fn kill(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for BackgroundTaskManager {
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &std::path::Path,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        let task = closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running,
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
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        let task = closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running,
            output_path: std::path::PathBuf::from("/tmp/output"),
        };
        self.tasks
            .write()
            .await
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }
    async fn kill_task(&self, task_id: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        self.tasks.write().await.remove(task_id);
        Ok(())
    }
    async fn get_task(&self, task_id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        self.tasks.read().await.get(task_id).cloned()
    }
}

fn test_session_manager() -> Arc<closeclaw_gateway::SessionManager> {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(closeclaw_gateway::SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: closeclaw_gateway::DmScope::default(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn test_config_manager() -> Arc<closeclaw_config::ConfigManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    Arc::new(
        closeclaw_config::ConfigManager::new(tmp.path().to_path_buf())
            .expect("ConfigManager::new should succeed"),
    )
}

fn test_mock_approval_flow() -> Arc<TokioMutex<ApprovalFlow>> {
    Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&test_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
    )))
}

fn test_tool_context() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

// --- process_output ---

#[test]
fn test_process_output_short_string() {
    let out = process_output("hello world");
    assert_eq!(out.inline, "hello world");
    assert!(out.persisted_path.is_none());
    assert_eq!(out.persisted_size, 0);
}

#[test]
fn test_process_output_exact_boundary() {
    let exact: String = "a".repeat(MAX_OUTPUT_CHARS);
    let out = process_output(&exact);
    assert_eq!(out.inline, exact);
    assert!(out.persisted_path.is_none());
}

#[test]
fn test_process_output_long_string_truncates() {
    let long: String = "x".repeat(MAX_OUTPUT_CHARS + 1000);
    let out = process_output(&long);
    // When persist succeeds, inline is a <persisted-output> reference
    assert!(out.inline.contains("<persisted-output"));
    assert!(out.persisted_path.is_some());
    assert_eq!(out.persisted_size, long.len());
    if let Some(ref p) = out.persisted_path {
        let _ = std::fs::remove_file(p);
    }
}

// --- persist_output ---

#[test]
fn test_persist_output_writes_file() {
    let path = persist_output("test persist data").unwrap();
    assert!(std::path::Path::new(&path).exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "test persist data");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_persist_output_cleans_up() {
    let path = persist_output("cleanup test").unwrap();
    assert!(std::path::Path::new(&path).exists());
    std::fs::remove_file(&path).unwrap();
    assert!(!std::path::Path::new(&path).exists());
}

// --- parse_timeout ---

#[test]
fn test_parse_timeout_default() {
    let args = serde_json::json!({});
    assert_eq!(parse_timeout(&args), 120_000);
}

#[test]
fn test_parse_timeout_custom() {
    let args = serde_json::json!({"timeout": 5000});
    assert_eq!(parse_timeout(&args), 5000);
}

#[test]
fn test_parse_timeout_clamped() {
    let args = serde_json::json!({"timeout": 900_000});
    assert_eq!(parse_timeout(&args), 600_000);
}

#[test]
fn test_parse_timeout_zero() {
    let args = serde_json::json!({"timeout": 0});
    assert_eq!(parse_timeout(&args), 0);
}

// --- resolve_cwd ---

#[test]
fn test_resolve_cwd_no_cwd_no_workdir() {
    let args = serde_json::json!({});
    let ctx = test_tool_context();
    let result = resolve_cwd(&args, &ctx);
    let expected = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string());
    assert_eq!(result, expected);
}

#[test]
fn test_resolve_cwd_with_cwd_arg() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("test").to_string_lossy().to_string();
    let args = serde_json::json!({"cwd": cwd});
    let ctx = test_tool_context();
    assert_eq!(
        resolve_cwd(&args, &ctx),
        tmp.path().join("test").to_string_lossy().to_string()
    );
}

// --- BashTool metadata ---

#[tokio::test]
async fn test_bash_tool_name_and_group() {
    let tool = BashTool::new(
        test_permission_engine(),
        test_bg_manager(),
        test_session_manager(),
        test_config_manager(),
        test_mock_approval_flow(),
    );
    assert_eq!(tool.name(), "Bash");
    assert_eq!(tool.group(), "bash");
}

#[tokio::test]
async fn test_bash_tool_flags() {
    let tool = BashTool::new(
        test_permission_engine(),
        test_bg_manager(),
        test_session_manager(),
        test_config_manager(),
        test_mock_approval_flow(),
    );
    let flags = tool.flags();
    assert!(flags.is_destructive);
    assert!(flags.is_expensive);
}

// --- input_schema ---

#[tokio::test]
async fn test_input_schema_command_required() {
    let tool = BashTool::new(
        test_permission_engine(),
        test_bg_manager(),
        test_session_manager(),
        test_config_manager(),
        test_mock_approval_flow(),
    );
    let schema = tool.input_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("command")));
}

#[tokio::test]
async fn test_input_schema_six_properties() {
    let tool = BashTool::new(
        test_permission_engine(),
        test_bg_manager(),
        test_session_manager(),
        test_config_manager(),
        test_mock_approval_flow(),
    );
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert_eq!(props.len(), 6);
    for name in &[
        "command",
        "timeout",
        "description",
        "run_in_background",
        "cwd",
        "dangerouslyDisableSandbox",
    ] {
        assert!(props.contains_key(*name), "missing property: {}", name);
    }
}

// ---------------------------------------------------------------------------
// Step 1.3: UT for run_in_background + auto-background paths
// (Refs: issue #814 — `run_in_background` must use `spawn()`, auto-bg must
// include `assistantAutoBackgrounded: true` + `backgroundTaskId`.)
// ---------------------------------------------------------------------------

// --- build_background_result ---

#[test]
fn test_build_background_result_has_task_id_and_output_path() {
    use closeclaw_tasks::{BackgroundTask, TaskState};
    let tmp = TempDir::new().unwrap();
    let output_path = tmp.path().join("x/output");
    let task = BackgroundTask {
        id: "task-abc-123".to_string(),
        command: "echo hi".to_string(),
        state: TaskState::Running,
        output_path,
    };
    let result = build_background_result(&task);
    assert_eq!(
        result.data["backgroundTaskId"],
        json!("task-abc-123"),
        "explicit-background result must expose backgroundTaskId"
    );
    assert_eq!(
        result.data["outputPath"],
        json!(tmp.path().join("x/output")),
        "explicit-background result must expose outputPath"
    );
}

#[test]
fn test_build_background_result_has_no_auto_backgrounded_flag() {
    use closeclaw_tasks::{BackgroundTask, TaskState};
    let tmp = TempDir::new().unwrap();
    let task = BackgroundTask {
        id: "task-no-auto".to_string(),
        command: "true".to_string(),
        state: TaskState::Running,
        output_path: tmp.path().join("y/output"),
    };
    let result = build_background_result(&task);
    // Explicit `run_in_background: true` must NOT set the auto flag.
    let flag = &result.data["assistantAutoBackgrounded"];
    assert!(
        flag.is_null() || flag == &json!(false),
        "explicit-background result must not set assistantAutoBackgrounded=true, got: {}",
        flag
    );
}

// --- build_auto_background_result ---

#[test]
fn test_build_auto_background_result_has_task_id_and_flag() {
    use closeclaw_tasks::{BackgroundTask, TaskState};
    let tmp = TempDir::new().unwrap();
    let output_path = tmp.path().join("z/output");
    let task = BackgroundTask {
        id: "task-auto-bg".to_string(),
        command: "sleep 10".to_string(),
        state: TaskState::Running,
        output_path,
    };
    let result = build_auto_background_result(&task);
    assert_eq!(
        result.data["backgroundTaskId"],
        json!("task-auto-bg"),
        "auto-background result must expose backgroundTaskId"
    );
    assert_eq!(
        result.data["outputPath"],
        json!(tmp.path().join("z/output")),
        "auto-background result must expose outputPath"
    );
    assert_eq!(
        result.data["assistantAutoBackgrounded"],
        json!(true),
        "auto-background result must set assistantAutoBackgrounded=true"
    );
}

// --- execute_command with run_in_background: true ---

#[tokio::test]
async fn test_execute_command_run_in_background_returns_background_task() {
    use closeclaw_tasks::TaskState;
    let bg_manager: Arc<BackgroundTaskManager> = Arc::new(BackgroundTaskManager::new());
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = {
        let b = Arc::clone(&bg_manager);
        b as Arc<dyn closeclaw_tasks::TaskManager>
    };

    let tmp = TempDir::new().unwrap();
    let result = execute_command(
        "echo run_in_bg",
        tmp.path().to_str().unwrap(),
        5_000,
        true,
        &bg_trait,
        None,
        None,
    )
    .await
    .expect("execute_command(run_in_background) should succeed");

    // Required fields per design (background-tasks.md)
    let task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("backgroundTaskId must be a non-null string");
    assert!(!task_id.is_empty(), "backgroundTaskId must not be empty");
    assert!(
        result.data["outputPath"].is_string(),
        "outputPath must be a string, got: {}",
        result.data["outputPath"]
    );

    // The explicit-background path must NOT set the auto flag.
    let flag = &result.data["assistantAutoBackgrounded"];
    assert!(
        flag.is_null() || flag == &json!(false),
        "explicit-background path must not set assistantAutoBackgrounded=true, got: {}",
        flag
    );

    // The task must be tracked by the manager (state: Running).
    let snapshot = bg_manager
        .get_task(task_id)
        .await
        .expect("background task must be tracked by the manager");
    assert_eq!(snapshot.id, task_id);
    assert_eq!(snapshot.command, "echo run_in_bg");
    assert_eq!(snapshot.state, TaskState::Running);
    assert!(bg_manager.is_running(task_id).await);

    // Wait for completion so the spawned tokio task is dropped before
    // the test exits (avoids leaving an orphan in CI).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if !bg_manager.is_running(task_id).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
}

#[tokio::test]
async fn test_execute_command_run_in_background_with_long_command() {
    // Verify that `run_in_background: true` does NOT pre-spawn a Child
    // via `spawn_sh_command` (which would fail for an invalid command
    // like `nonexistent_xyz`). With the new `spawn()` path, the task is
    // registered immediately and the command runs in the background.
    let bg_manager: Arc<BackgroundTaskManager> = Arc::new(BackgroundTaskManager::new());
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = {
        let b = Arc::clone(&bg_manager);
        b as Arc<dyn closeclaw_tasks::TaskManager>
    };
    let tmp = TempDir::new().unwrap();
    let result = execute_command(
        "nonexistent_xyz_abcdef_12345",
        tmp.path().to_str().unwrap(),
        5_000,
        true,
        &bg_trait,
        None,
        None,
    )
    .await
    .expect("execute_command(run_in_background) should succeed even for unknown commands");
    let task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("backgroundTaskId must be a string");
    assert!(!task_id.is_empty());
    // Task is registered as Running (the actual command failure is
    // observed asynchronously in the spawned task).
    assert!(bg_manager.is_running(task_id).await);

    // Wait for the spawned task to settle into a Failed state.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if !bg_manager.is_running(task_id).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
}

// --- handle_foreground_result: auto-background path ---

#[tokio::test]
async fn test_handle_foreground_result_auto_backgrounds_on_timeout() {
    let bg_manager: Arc<BackgroundTaskManager> = Arc::new(BackgroundTaskManager::new());
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = {
        let b = Arc::clone(&bg_manager);
        b as Arc<dyn closeclaw_tasks::TaskManager>
    };
    // Spawn a child that will outlast the bg_timeout.
    let tmp = TempDir::new().unwrap();
    let child = spawn_sh_command("sleep 5", tmp.path().to_str().unwrap()).expect("spawn sleep");
    // Wrap the child in the shared `Arc<Mutex<Option<_>>>` slot that
    // `handle_foreground_result` now expects (Step 1.4 refactor:
    // the slot is the same one `BashKillHandle` would observe).
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    // Use a tiny bg_timeout so the auto-background path is triggered
    // almost immediately. This is the exact branch Step 1.2 unlocked:
    // `backgroundize(child, command)` is now called WITHOUT a cwd arg.
    let result = handle_foreground_result(
        child_arc,
        "sleep 5",
        std::time::Duration::from_millis(100),
        &bg_trait,
    )
    .await
    .expect("auto-background path should succeed");

    let task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("auto-bg result must expose backgroundTaskId");
    assert!(!task_id.is_empty());
    assert_eq!(
        result.data["assistantAutoBackgrounded"],
        json!(true),
        "auto-bg result must set assistantAutoBackgrounded=true"
    );
    assert!(
        result.data["outputPath"].is_string(),
        "auto-bg result must expose outputPath"
    );

    // The task is now managed by the manager (state: Running).
    assert!(bg_manager.is_running(task_id).await);

    // Clean up: kill the orphan sleep so it does not linger after the
    // test process exits.
    let _ = bg_manager.kill(task_id).await;
}

#[tokio::test]
async fn test_handle_foreground_result_returns_foreground_on_success() {
    // Control test: when the child completes before bg_timeout, the
    // result must be a foreground result (no background fields).
    let bg_manager: Arc<BackgroundTaskManager> = Arc::new(BackgroundTaskManager::new());
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = {
        let b = Arc::clone(&bg_manager);
        b as Arc<dyn closeclaw_tasks::TaskManager>
    };
    let tmp = TempDir::new().unwrap();
    let child = spawn_sh_command("true", tmp.path().to_str().unwrap()).expect("spawn true");
    // Wrap the child in the shared slot — `handle_foreground_result`
    // extracts stdout/stderr and then takes the child for `wait()`.
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    let result = handle_foreground_result(
        child_arc,
        "true",
        std::time::Duration::from_secs(5),
        &bg_trait,
    )
    .await
    .expect("foreground path should succeed");

    assert_eq!(
        result.data["exitCode"],
        json!(0),
        "foreground result must include exitCode=0"
    );
    assert!(
        result.data["backgroundTaskId"].is_null(),
        "foreground result must not expose backgroundTaskId, got: {}",
        result.data["backgroundTaskId"]
    );
    let flag = &result.data["assistantAutoBackgrounded"];
    assert!(
        flag.is_null() || flag == &json!(false),
        "foreground result must not set assistantAutoBackgrounded=true, got: {}",
        flag
    );
}
