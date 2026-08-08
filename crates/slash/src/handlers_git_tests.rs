//! Tests for /git subcommand routing (split from handlers_tests.rs to stay
//! under the 1000-line file limit).

use std::path::Path;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers::WorkdirHandler;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::session_manager::SessionManager;

// ── Helpers (duplicated from handlers_tests to avoid cross-module deps) ─────

fn make_workdir_session_manager() -> Arc<SessionManager> {
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None, // storage
        None, // workspace_dir
        ReasoningLevel::default(),
    ))
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "git-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let account_id: Option<&str> = None;
    sm.find_or_create("feishu", &msg, account_id)
        .await
        .expect("session")
}

/// Set a session's workdir to `path` by directly mutating
/// the ConversationSession via the SessionManager.
async fn set_session_workdir(sm: &Arc<SessionManager>, sid: &str, path: &Path) {
    let conv = sm
        .get_conversation_session(sid)
        .await
        .expect("session should exist");
    let mut cs = conv.write().await;
    cs.set_workdir(path.to_path_buf());
}

/// Assert a SlashResult is Exec with the expected command prefix.
fn assert_exec_command(result: SlashResult, expected_prefix: &str) -> String {
    match result {
        SlashResult::Exec {
            command,
            requires_permission: _,
        } => {
            assert!(
                command.starts_with(expected_prefix),
                "expected command starting with '{expected_prefix}', got: {command}"
            );
            command
        }
        other => panic!("expected Exec, got {other:?}"),
    }
}

// ── /git status tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_git_status_in_non_git_repo() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("status", &ctx).await, "git ");
    assert_eq!(command, "git status");
}

#[tokio::test]
async fn test_git_status_in_git_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::process::Command::new("git")
        .args(["init", repo_path.to_str().unwrap()])
        .output()
        .expect("git init failed");
    std::fs::write(repo_path.join(".gitkeep"), "").unwrap();
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "add", "."])
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "commit", "-m", "init"])
        .output()
        .expect("git commit failed");

    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    set_session_workdir(&sm, &sid, &repo_path).await;

    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("status", &ctx).await, "git ");
    assert_eq!(command, "git status");
}

#[tokio::test]
async fn test_git_no_args_returns_usage() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("用法"), "got: {t}"),
        other => panic!("expected Reply with usage, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_unknown_subcommand_routes_to_exec() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("unknown", &ctx).await, "git ");
    assert_eq!(command, "git unknown");
}

#[tokio::test]
async fn test_git_status_extra_args_ignored() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("status --porcelain", &ctx).await, "git ");
    assert_eq!(command, "git status --porcelain");
}

#[tokio::test]
async fn test_git_status_no_session() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("status", &ctx).await, "git ");
    assert_eq!(command, "git status");
}

#[tokio::test]
async fn test_git_status_in_git_repo_with_uncommitted_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::process::Command::new("git")
        .args(["init", repo_path.to_str().unwrap()])
        .output()
        .expect("git init failed");
    std::fs::write(repo_path.join(".gitkeep"), "").unwrap();
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "add", "."])
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "commit", "-m", "init"])
        .output()
        .expect("git commit failed");

    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    set_session_workdir(&sm, &sid, &repo_path).await;

    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    let command = assert_exec_command(h.handle("status", &ctx).await, "git ");
    assert_eq!(command, "git status");
}

// ── /git subcommand routing tests ──────────────────────────────────────────

#[tokio::test]
async fn test_git_write_subcommands_route_to_exec() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    for sub in [
        "commit -m \"test\"",
        "push origin main",
        "merge feature",
        "rebase main",
    ] {
        let command = assert_exec_command(h.handle(sub, &ctx).await, "git ");
        assert_eq!(command, format!("git {sub}"));
    }
}

#[tokio::test]
async fn test_git_subcommands_route_to_exec() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    for sub in ["log", "diff", "branch", "show", "status"] {
        let command = assert_exec_command(h.handle(sub, &ctx).await, "git ");
        assert_eq!(command, format!("git {sub}"));
    }
    let command = assert_exec_command(h.handle("log --oneline -5", &ctx).await, "git ");
    assert_eq!(command, "git log --oneline -5");
    let command = assert_exec_command(h.handle("diff HEAD~1", &ctx).await, "git ");
    assert_eq!(command, "git diff HEAD~1");
}

#[tokio::test]
async fn test_git_requires_permission() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    assert!(
        !h.requires_permission(),
        "WorkdirHandler should not require permission at handler level"
    );
}

// ── requires_permission field tests ─────────────────────────────────────────

#[tokio::test]
async fn test_git_read_write_requires_permission() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("status", &ctx).await {
        SlashResult::Exec {
            command,
            requires_permission,
        } => {
            assert_eq!(command, "git status");
            assert!(!requires_permission);
        }
        other => panic!("expected Exec, got {other:?}"),
    }
    match h.handle("commit -m \"test\"", &ctx).await {
        SlashResult::Exec {
            command,
            requires_permission,
        } => {
            assert_eq!(command, "git commit -m \"test\"");
            assert!(requires_permission);
        }
        other => panic!("expected Exec, got {other:?}"),
    }
}
