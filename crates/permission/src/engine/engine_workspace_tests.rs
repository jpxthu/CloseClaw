use std::path::PathBuf;

use tempfile::TempDir;

use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::actions::ActionBuilder;
use crate::rules::{RuleBuilder, RuleSetBuilder};

fn make_engine() -> PermissionEngine {
    let tmp = TempDir::new().unwrap();
    make_engine_with_root(tmp.path().to_path_buf())
}

fn ws_path(root: &std::path::Path, agent: &str, user: &str, file: &str) -> String {
    root.join("workspaces")
        .join(agent)
        .join(user)
        .join(file)
        .to_string_lossy()
        .into_owned()
}

fn make_file_request(agent: &str, user_id: &str, path: &str, op: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: agent.to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: agent.to_string(),
            path: path.to_string(),
            op: op.to_string(),
        },
    }
}

fn make_exec_request(agent: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "test-user".to_string(),
            agent: agent.to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::CommandExec {
            agent: agent.to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        },
    }
}

fn make_engine_with_root(data_root: PathBuf) -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("deny-write")
                .subject_agent("test-agent")
                .deny()
                .action(
                    ActionBuilder::file("write", vec!["/etc/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    PermissionEngine::new(ruleset, data_root)
}

#[test]
fn test_workspace_read_allowed() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "test-agent", "test-user", "file.txt"),
        "read",
    );
    let result = engine.evaluate(request, None);
    assert!(matches!(result, PermissionResponse::Allowed { .. }));
}

#[test]
fn test_workspace_write_allowed() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "test-agent", "test-user", "file.txt"),
        "write",
    );
    let result = engine.evaluate(request, None);
    assert!(matches!(result, PermissionResponse::Allowed { .. }));
}

#[test]
fn test_workspace_outside_read_denied() {
    let engine = make_engine();
    let request = make_file_request("test-agent", "test-user", "/etc/passwd", "read");
    let result = engine.evaluate(request, None);
    // Outside workspace → normal rule evaluation → no matching rule → default Deny
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}

#[test]
fn test_workspace_outside_write_denied() {
    let engine = make_engine();
    let request = make_file_request("test-agent", "test-user", "/etc/passwd", "write");
    let result = engine.evaluate(request, None);
    // Outside workspace → normal rule evaluation → deny-write rule → Denied
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}

#[test]
fn test_workspace_exec_not_auto_authorized() {
    let engine = make_engine();
    let request = make_exec_request("test-agent");
    let result = engine.evaluate(request, None);
    // exec does not trigger workspace auto-authorization; falls to default Deny
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}

#[test]
fn test_workspace_path_traversal_blocked() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "test-agent", "test-user", "../../../etc/passwd"),
        "read",
    );
    let result = engine.evaluate(request, None);
    // Normalized to /etc/passwd, outside workspace → falls to default Deny
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}

#[test]
fn test_workspace_normalize_path_in_workspace() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "test-agent", "test-user", "foo/../bar/file.txt"),
        "read",
    );
    let result = engine.evaluate(request, None);
    assert!(matches!(result, PermissionResponse::Allowed { .. }));
}

#[test]
fn test_workspace_normalize_path_outside_workspace() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "other-agent", "test-user", "file.txt"),
        "read",
    );
    let result = engine.evaluate(request, None);
    // agent is "test-agent" but path belongs to "other-agent" → outside workspace
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}

#[test]
fn test_workspace_owner_allowed() {
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: ws_path(tmp.path(), "test-agent", "owner", "file.txt"),
            op: "read".to_string(),
        },
    };
    let result = engine.evaluate(request, None);
    assert!(matches!(result, PermissionResponse::Allowed { .. }));
}

#[test]
fn test_workspace_user_prefix_boundary() {
    // Security fix: test-user must NOT match test-user2
    let tmp = TempDir::new().unwrap();
    let engine = make_engine_with_root(tmp.path().to_path_buf());
    let request = make_file_request(
        "test-agent",
        "test-user",
        &ws_path(tmp.path(), "test-agent", "test-user2", "file.txt"),
        "read",
    );
    let result = engine.evaluate(request, None);
    // test-user2 is NOT the same as test-user → outside workspace
    assert!(matches!(result, PermissionResponse::Denied { .. }));
}
