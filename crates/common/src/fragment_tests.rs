//! Tests for `FragmentContext` and `PromptFragmentProvider`.

use super::*;
use crate::bootstrap::BootstrapMode;

// ---------------------------------------------------------------------------
// FragmentContext
// ---------------------------------------------------------------------------

#[test]
fn test_fragment_context_test_default() {
    let ctx = FragmentContext::test_default();
    assert_eq!(ctx.agent_id, "");
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
    assert!(ctx.workdir.is_dir());
}

#[test]
fn test_fragment_context_agent_id() {
    let ctx = FragmentContext {
        agent_id: "agent-42".to_string(),
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.agent_id, "agent-42");
}

#[test]
fn test_fragment_context_bootstrap_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
}

#[test]
fn test_fragment_context_workdir() {
    let ctx = FragmentContext {
        workdir: std::path::PathBuf::from("/tmp/workspace"),
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.workdir, std::path::PathBuf::from("/tmp/workspace"));
}

#[test]
fn test_fragment_context_all_fields() {
    let ctx = FragmentContext {
        agent_id: "my-agent".to_string(),
        bootstrap_mode: BootstrapMode::Minimal,
        workdir: std::path::PathBuf::from("/home/user/project"),
    };
    assert_eq!(ctx.agent_id, "my-agent");
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(ctx.workdir, std::path::PathBuf::from("/home/user/project"));
}

#[test]
fn test_fragment_context_clone() {
    let ctx = FragmentContext {
        agent_id: "clone-test".to_string(),
        bootstrap_mode: BootstrapMode::Minimal,
        workdir: std::path::PathBuf::from("/clone"),
    };
    let cloned = ctx.clone();
    assert_eq!(ctx.agent_id, cloned.agent_id);
    assert_eq!(ctx.bootstrap_mode, cloned.bootstrap_mode);
    assert_eq!(ctx.workdir, cloned.workdir);
}

#[test]
fn test_fragment_context_debug() {
    let ctx = FragmentContext {
        agent_id: "dbg".to_string(),
        ..FragmentContext::test_default()
    };
    let dbg = format!("{:?}", ctx);
    assert!(dbg.contains("dbg"));
}

#[test]
fn test_fragment_context_empty_agent_id_boundary() {
    let ctx = FragmentContext {
        agent_id: String::new(),
        ..FragmentContext::test_default()
    };
    assert!(ctx.agent_id.is_empty());
}

#[test]
fn test_fragment_context_minimal_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Minimal,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Minimal);
}

#[test]
fn test_fragment_context_full_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
}
