//! Tests for session-related slash handlers.
//!
//! Verifies:
//! - `/stop` without args → `cascade = false` (Step 1.2 fix)
//! - `/stop --cascade` → `cascade = true`
//! - `/stop --force` → `force = true`

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_session::StopHandler;
use closeclaw_common::slash_router::SlashResult;

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

// ── /stop without args → cascade = false ───────────────────────────────────

#[tokio::test]
async fn test_stop_no_args_cascade_false() {
    let handler = StopHandler;
    let ctx = dummy_ctx();
    let result = handler.handle("", &ctx).await;
    match result {
        SlashResult::Stop { cascade, force } => {
            assert!(!cascade, "/stop without args should default cascade=false");
            assert!(!force, "/stop without args should default force=false");
        }
        other => panic!("expected SlashResult::Stop, got {:?}", other),
    }
}

// ── /stop --cascade → cascade = true ───────────────────────────────────────

#[tokio::test]
async fn test_stop_with_cascade_flag() {
    let handler = StopHandler;
    let ctx = dummy_ctx();
    let result = handler.handle("--cascade", &ctx).await;
    match result {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "/stop --cascade should set cascade=true");
            assert!(!force, "force should remain false");
        }
        other => panic!("expected SlashResult::Stop, got {:?}", other),
    }
}

// ── /stop --force → force = true ───────────────────────────────────────────

#[tokio::test]
async fn test_stop_with_force_flag() {
    let handler = StopHandler;
    let ctx = dummy_ctx();
    let result = handler.handle("--force", &ctx).await;
    match result {
        SlashResult::Stop { cascade, force } => {
            assert!(!cascade, "cascade should remain false");
            assert!(force, "/stop --force should set force=true");
        }
        other => panic!("expected SlashResult::Stop, got {:?}", other),
    }
}

// ── /stop --cascade --force → both true ────────────────────────────────────

#[tokio::test]
async fn test_stop_with_both_flags() {
    let handler = StopHandler;
    let ctx = dummy_ctx();
    let result = handler.handle("--cascade --force", &ctx).await;
    match result {
        SlashResult::Stop { cascade, force } => {
            assert!(cascade, "/stop --cascade --force should set cascade=true");
            assert!(force, "/stop --cascade --force should set force=true");
        }
        other => panic!("expected SlashResult::Stop, got {:?}", other),
    }
}

// ── /stop with unknown args → defaults ─────────────────────────────────────

#[tokio::test]
async fn test_stop_unknown_args_ignored() {
    let handler = StopHandler;
    let ctx = dummy_ctx();
    let result = handler.handle("--unknown", &ctx).await;
    match result {
        SlashResult::Stop { cascade, force } => {
            assert!(!cascade, "unknown args should not affect cascade");
            assert!(!force, "unknown args should not affect force");
        }
        other => panic!("expected SlashResult::Stop, got {:?}", other),
    }
}

// ── Handler metadata tests ─────────────────────────────────────────────────

#[test]
fn test_stop_handler_commands() {
    let handler = StopHandler;
    assert_eq!(handler.commands(), &["stop"]);
}

#[test]
fn test_stop_handler_immediate() {
    let handler = StopHandler;
    assert!(handler.immediate("stop"), "/stop should be immediate");
}
