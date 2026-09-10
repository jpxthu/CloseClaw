//! Level 2 permission check tests for the dispatcher.
//!
//! Tests command-level (bash), file-operation (file_ops), and config-write
//! permission checks that are executed by the dispatcher's `ToolRegistryExecutor`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use closeclaw_common::tool_trait::{ToolCallError, ToolResult};

use super::*;

use closeclaw_common::ToolContext;

use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{Action, Effect, Rule};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_permission::Defaults;
use tokio::sync::Mutex as TokioMutex;

// ---------------------------------------------------------------------------
// Test tools
// ---------------------------------------------------------------------------

/// A tool with group="bash" for testing command-level permissions.
struct BashEchoTool;

#[async_trait]
impl crate::Tool for BashEchoTool {
    fn name(&self) -> &str {
        "BashEcho"
    }
    fn group(&self) -> &str {
        "bash"
    }
    fn summary(&self) -> String {
        "bash echo".into()
    }
    fn detail(&self) -> String {
        "bash echo".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["command"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        Ok(ToolResult {
            data: args,
            new_messages: vec![],
            context_modifier: None,
        })
    }
    fn flags(&self) -> crate::ToolFlags {
        crate::ToolFlags {
            is_destructive: true,
            ..Default::default()
        }
    }
}

/// A read-only tool with group="file_ops" for testing file read permissions.
struct FileReadTool;

#[async_trait]
impl crate::Tool for FileReadTool {
    fn name(&self) -> &str {
        "FileRead"
    }
    fn group(&self) -> &str {
        "file_ops"
    }
    fn summary(&self) -> String {
        "file read".into()
    }
    fn detail(&self) -> String {
        "file read".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        Ok(ToolResult {
            data: args,
            new_messages: vec![],
            context_modifier: None,
        })
    }
    fn flags(&self) -> crate::ToolFlags {
        crate::ToolFlags {
            is_read_only: true,
            is_concurrency_safe: true,
            ..Default::default()
        }
    }
}

/// A write tool with group="file_ops" for testing file write permissions.
struct FileWriteTool;

#[async_trait]
impl crate::Tool for FileWriteTool {
    fn name(&self) -> &str {
        "FileWrite"
    }
    fn group(&self) -> &str {
        "file_ops"
    }
    fn summary(&self) -> String {
        "file write".into()
    }
    fn detail(&self) -> String {
        "file write".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        Ok(ToolResult {
            data: args,
            new_messages: vec![],
            context_modifier: None,
        })
    }
    fn flags(&self) -> crate::ToolFlags {
        crate::ToolFlags {
            is_read_only: false,
            is_destructive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (duplicated from dispatcher_tests.rs for module isolation)
// ---------------------------------------------------------------------------

/// ToolContext for permission tests.
fn make_perm_ctx(session_id: Option<&str>) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".into(),
        workdir: None,
        session_id: session_id.map(String::from),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

/// Allow rule for a tool group.
fn make_allow_rule(agent: &str, skill: &str) -> Rule {
    Rule {
        name: format!("allow-{skill}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ToolCall {
            skill: skill.to_string(),
            methods: vec!["call".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build PermDeps with configurable defaults for file and command dimensions.
fn make_l2_perm_deps(rules: Vec<Rule>) -> PermDeps {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Allow, // Level 1: allow all tools
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            exec: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    let perm = Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ));
    let sm = {
        use closeclaw_gateway::GatewayConfig;
        use closeclaw_session::persistence::ReasoningLevel;
        Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                ..Default::default()
            },
            None,
            None,
            ReasoningLevel::default(),
        ))
    };
    let cm = {
        let tmp = tempfile::TempDir::new().unwrap();
        Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("ConfigManager::new should succeed"),
        )
    };
    let af = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        closeclaw_permission::RuleSet::default(),
    )));
    (perm, sm, cm, af)
}

/// Build PermDeps with deny-all approval flow for hard denial tests.
fn make_l2_perm_deps_deny(rules: Vec<Rule>) -> PermDeps {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Allow, // Level 1: allow all tools
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            exec: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    let perm = Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ));
    let sm = {
        use closeclaw_gateway::GatewayConfig;
        use closeclaw_session::persistence::ReasoningLevel;
        Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                ..Default::default()
            },
            None,
            None,
            ReasoningLevel::default(),
        ))
    };
    let cm = {
        let tmp = tempfile::TempDir::new().unwrap();
        Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("ConfigManager::new should succeed"),
        )
    };
    let af = Arc::new(TokioMutex::new(ApprovalFlow::new_deny_all(
        Arc::clone(&sm) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        closeclaw_permission::RuleSet::default(),
    )));
    (perm, sm, cm, af)
}

/// Allow rule for command execution.
fn make_cmd_allow_rule(agent: &str, cmd: &str) -> Rule {
    Rule {
        name: format!("allow-cmd-{cmd}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::Command {
            command: cmd.to_string(),
            args: Default::default(),
        }],
        template: None,
        priority: 0,
    }
}

/// Allow rule for file operations.
fn make_file_allow_rule(agent: &str, path_glob: &str, op: &str) -> Rule {
    Rule {
        name: format!("allow-file-{op}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: op.to_string(),
            paths: vec![path_glob.to_string()],
        }],
        template: None,
        priority: 0,
    }
}

// ---------------------------------------------------------------------------
// Bash command-level tests
// ---------------------------------------------------------------------------

/// Bash tool: Level 1 allowed, Level 2 (command) denied → denied result.
#[tokio::test]
async fn test_level2_bash_command_denied() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(BashEchoTool).await.unwrap();

    // Level 1 (tool_call): Allow "bash" group
    // Level 2 (exec): Deny all (default) + deny-all approval flow
    let deps = make_l2_perm_deps_deny(vec![make_allow_rule("test-agent", "bash")]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-bash-1".into(),
        tool_name: "BashEcho".into(),
        args: serde_json::json!({"command": "rm -rf /"}),
        file_path: None,
        is_concurrency_safe: false,
    };
    let result = executor.execute(&call).await;
    // Level 2 command denied → error in result
    assert!(
        result.data.get("error").is_some() || result.data.get("status").is_some(),
        "Level 2 command denial should produce error or status, got: {:?}",
        result.data
    );
}

/// Bash tool: Level 1 allowed, Level 2 (command) allowed → tool executes.
#[tokio::test]
async fn test_level2_bash_command_allowed() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(BashEchoTool).await.unwrap();

    let deps = make_l2_perm_deps(vec![
        make_allow_rule("test-agent", "bash"),
        make_cmd_allow_rule("test-agent", "echo"),
    ]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-bash-2".into(),
        tool_name: "BashEcho".into(),
        args: serde_json::json!({"command": "echo hello"}),
        file_path: None,
        is_concurrency_safe: false,
    };
    let result = executor.execute(&call).await;
    // Level 2 command allowed → tool executes normally (no error)
    assert!(
        result.data.get("error").is_none(),
        "tool should execute when Level 2 command is allowed, got: {:?}",
        result.data
    );
    assert_eq!(
        result.data["command"], "echo hello",
        "tool should execute and return command arg"
    );
}

/// Bash tool without command arg → Level 2 check skipped, tool executes.
#[tokio::test]
async fn test_level2_bash_no_command_skips_check() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(BashEchoTool).await.unwrap();

    // Level 1 allowed, but no command in args → Level 2 skipped
    let deps = make_l2_perm_deps(vec![make_allow_rule("test-agent", "bash")]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-bash-3".into(),
        tool_name: "BashEcho".into(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: false,
    };
    let result = executor.execute(&call).await;
    // No command → Level 2 skipped → tool executes with empty args
    assert!(
        result.data.get("error").is_none(),
        "tool should execute when no command arg is provided, got: {:?}",
        result.data
    );
}

// ---------------------------------------------------------------------------
// FileOps file-operation tests
// ---------------------------------------------------------------------------

/// FileOps read tool: Level 1 allowed, Level 2 (file read) denied → denied.
#[tokio::test]
async fn test_level2_fileops_read_denied() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FileReadTool).await.unwrap();

    // Level 1: Allow "file_ops", Level 2: Deny all file ops + deny-all approval flow
    let deps = make_l2_perm_deps_deny(vec![make_allow_rule("test-agent", "file_ops")]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-fo-1".into(),
        tool_name: "FileRead".into(),
        args: serde_json::json!({"path": "/etc/passwd"}),
        file_path: Some("/etc/passwd".into()),
        is_concurrency_safe: true,
    };
    let result = executor.execute(&call).await;
    // Level 2 file read denied → error or status in result
    assert!(
        result.data.get("error").is_some() || result.data.get("status").is_some(),
        "Level 2 file read denial should produce error or status, got: {:?}",
        result.data
    );
}

/// FileOps read tool: Level 1 allowed, Level 2 (file read) allowed → executes.
#[tokio::test]
async fn test_level2_fileops_read_allowed() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FileReadTool).await.unwrap();

    let deps = make_l2_perm_deps(vec![
        make_allow_rule("test-agent", "file_ops"),
        make_file_allow_rule("test-agent", "/tmp/**", "read"),
    ]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-fo-2".into(),
        tool_name: "FileRead".into(),
        args: serde_json::json!({"path": "/tmp/test.txt"}),
        file_path: Some("/tmp/test.txt".into()),
        is_concurrency_safe: true,
    };
    let result = executor.execute(&call).await;
    assert_eq!(
        result.data["path"], "/tmp/test.txt",
        "tool should execute when Level 2 file read is allowed"
    );
}

/// FileOps write tool: Level 1 allowed, Level 2 (file write) denied → denied.
#[tokio::test]
async fn test_level2_fileops_write_denied() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FileWriteTool).await.unwrap();

    let deps = make_l2_perm_deps_deny(vec![make_allow_rule("test-agent", "file_ops")]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-fo-3".into(),
        tool_name: "FileWrite".into(),
        args: serde_json::json!({"path": "/tmp/out.txt", "content": "data"}),
        file_path: Some("/tmp/out.txt".into()),
        is_concurrency_safe: false,
    };
    let result = executor.execute(&call).await;
    assert!(
        result.data.get("error").is_some() || result.data.get("status").is_some(),
        "Level 2 file write denial should produce error or status, got: {:?}",
        result.data
    );
}

/// FileOps write tool: Level 1 allowed, Level 2 (file write) allowed → executes.
#[tokio::test]
async fn test_level2_fileops_write_allowed() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FileWriteTool).await.unwrap();

    let deps = make_l2_perm_deps(vec![
        make_allow_rule("test-agent", "file_ops"),
        make_file_allow_rule("test-agent", "/tmp/**", "write"),
    ]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-fo-4".into(),
        tool_name: "FileWrite".into(),
        args: serde_json::json!({"path": "/tmp/out.txt", "content": "data"}),
        file_path: Some("/tmp/out.txt".into()),
        is_concurrency_safe: false,
    };
    let result = executor.execute(&call).await;
    assert_eq!(
        result.data["path"], "/tmp/out.txt",
        "tool should execute when Level 2 file write is allowed"
    );
}

/// FileOps tool without path arg → Level 2 check skipped, tool executes.
#[tokio::test]
async fn test_level2_fileops_no_path_skips_check() {
    let registry = Arc::new(crate::ToolRegistryImpl::new());
    registry.register(FileReadTool).await.unwrap();

    let deps = make_l2_perm_deps(vec![make_allow_rule("test-agent", "file_ops")]);
    let executor = ToolRegistryExecutor::new(registry, make_perm_ctx(None)).with_perm_deps(deps);

    let call = PendingToolCall {
        id: "l2-fo-5".into(),
        tool_name: "FileRead".into(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: true,
    };
    let result = executor.execute(&call).await;
    // No path → Level 2 skipped → tool executes with empty args
    assert!(
        result.data.get("error").is_none(),
        "tool should execute when no path arg is provided, got: {:?}",
        result.data
    );
}
