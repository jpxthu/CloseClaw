//! Sandbox module tests

use super::*;
use crate::permission::{Defaults, PermissionRequest, PermissionRequestBody, RuleSet};

#[test]
fn test_sandbox_state_default() {
    let state = SandboxState::default();
    assert_eq!(state, SandboxState::Unstarted);
}

#[test]
fn test_sandbox_error_display() {
    let ipc_err = SandboxError::IpcTimeout;
    let ipc_msg = ipc_err.to_string();
    assert!(
        ipc_msg.contains("timeout") || ipc_msg.contains("IPC"),
        "IpcTimeout display should contain 'timeout' or 'IPC': {}",
        ipc_msg
    );

    let process_err = SandboxError::ProcessError("engine died".to_string());
    let process_msg = process_err.to_string();
    assert!(
        process_msg.contains("engine died"),
        "ProcessError display should contain the message: {}",
        process_msg
    );

    let invalid_state = SandboxError::InvalidState {
        state: SandboxState::Running,
    };
    let invalid_msg = invalid_state.to_string();
    assert!(
        invalid_msg.contains("Running"),
        "InvalidState display should contain the state: {}",
        invalid_msg
    );
}

// -------------------------------------------------------------------------
// Sandbox construction & builder
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_sandbox_new_state_is_unstarted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let sandbox = Sandbox::new(&path);
    assert_eq!(sandbox.state().await, SandboxState::Unstarted);
}

#[tokio::test]
async fn test_sandbox_with_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let custom_policy = SecurityPolicy::default();
    let _sandbox = Sandbox::new(&path).with_policy(custom_policy);
    // Policy is stored in the sandbox; we verify construction completes
    // without panic and state remains accessible
}

// -------------------------------------------------------------------------
// State accessors
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_sandbox_state_unstarted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let sandbox = Sandbox::new(&path);
    assert_eq!(sandbox.state().await, SandboxState::Unstarted);
}

// -------------------------------------------------------------------------
// Error paths — InvalidState on non-Running states
// -------------------------------------------------------------------------

fn dummy_request() -> PermissionRequest {
    PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/tmp/test".to_string(),
        op: "read".to_string(),
    })
}

fn dummy_rules() -> RuleSet {
    RuleSet {
        version: "1.0".to_string(),
        rules: vec![],
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: Default::default(),
    }
}

#[tokio::test]
async fn test_sandbox_evaluate_invalid_state_unstarted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let sandbox = Sandbox::new(&path);
    let result = sandbox.evaluate(dummy_request()).await;
    let err = result.unwrap_err();
    match err {
        SandboxError::InvalidState { state } => {
            assert_eq!(state, SandboxState::Unstarted);
        }
        other => panic!("expected InvalidState, got {:?}", other),
    }
}

#[tokio::test]
async fn test_sandbox_reload_rules_invalid_state_unstarted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let sandbox = Sandbox::new(&path);
    let result = sandbox.reload_rules(dummy_rules()).await;
    let err = result.unwrap_err();
    match err {
        SandboxError::InvalidState { state } => {
            assert_eq!(state, SandboxState::Unstarted);
        }
        other => panic!("expected InvalidState, got {:?}", other),
    }
}

#[tokio::test]
async fn test_sandbox_shutdown_unstarted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let mut sandbox = Sandbox::new(&path);
    sandbox.shutdown().await;
    assert_eq!(sandbox.state().await, SandboxState::Shutdown);
}

#[tokio::test]
async fn test_sandbox_evaluate_invalid_state_shutdown() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let mut sandbox = Sandbox::new(&path);
    sandbox.shutdown().await;
    assert_eq!(sandbox.state().await, SandboxState::Shutdown);
    let result = sandbox.evaluate(dummy_request()).await;
    let err = result.unwrap_err();
    match err {
        SandboxError::InvalidState { state } => {
            assert_eq!(state, SandboxState::Shutdown);
        }
        other => panic!("expected InvalidState, got {:?}", other),
    }
}

// -------------------------------------------------------------------------
// SandboxError From<std::io::Error>
// -------------------------------------------------------------------------

#[test]
fn test_sandbox_error_from_io_error() {
    use std::io;
    let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
    let sandbox_err: SandboxError = SandboxError::from(io_err);
    match sandbox_err {
        SandboxError::Ipc(_) => {}
        other => panic!("expected Ipc variant, got {:?}", other),
    }
}

// -------------------------------------------------------------------------
// Drop
// -------------------------------------------------------------------------

#[test]
fn test_sandbox_drop_unstarted_no_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sandbox.sock");
    let _sandbox = Sandbox::new(&path);
    // drop should not panic
}

// -------------------------------------------------------------------------
// SandboxState variants
// -------------------------------------------------------------------------

#[test]
fn test_sandbox_state_crashed_debug() {
    let state = SandboxState::Crashed { exit_code: Some(1) };
    let debug = format!("{:?}", state);
    assert!(debug.contains("Crashed"));
    assert!(debug.contains("1"));
}

#[test]
fn test_sandbox_state_crashed_clone() {
    let state = SandboxState::Crashed {
        exit_code: Some(42),
    };
    let cloned = state.clone();
    assert_eq!(state, cloned);
}

#[test]
fn test_sandbox_state_crashed_partial_eq() {
    let a = SandboxState::Crashed { exit_code: Some(1) };
    let b = SandboxState::Crashed { exit_code: Some(1) };
    let c = SandboxState::Crashed { exit_code: None };
    assert_eq!(a, b);
    assert_ne!(a, c);
}
