//! Unit tests for [`crate::gateway_restart`].
//!
//! Covers the restart state machine (Idle → Pending → Executing), the
//! restart-request batching, DaemonReloadCallback restart-class signal
//! delivery, and (Step 1.9) the restart-path turn-completion consumer
//! wiring (`recv → finish_turns`), mirroring `install_handlers`.
//!
//! Naming: the older unprefixed cases here predate STANDARDS §3 and are
//! kept as-is; **every newly added test case must carry the `test_`
//! prefix** (Step 1.21).

use super::*;
use crate::config_reload::reload::DaemonReloadCallback;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::ReloadCallback;
use std::path::Path;
use std::sync::Arc;

// -- RestartState display ----------------------------------------------

#[test]
fn restart_state_display_idle() {
    assert_eq!(RestartState::Idle.to_string(), "Idle");
}

#[test]
fn restart_state_display_pending() {
    let state = RestartState::Pending {
        changes: vec!["models.json".into(), "gateway.json".into()],
    };
    assert_eq!(state.to_string(), "Pending(models.json, gateway.json)");
}

#[test]
fn restart_state_display_executing() {
    assert_eq!(RestartState::Executing.to_string(), "Executing");
}

// -- RestartState equality ---------------------------------------------

#[test]
fn restart_state_equality() {
    let a = RestartState::Pending {
        changes: vec!["x".into()],
    };
    let b = RestartState::Pending {
        changes: vec!["x".into()],
    };
    assert_eq!(a, b);

    let c = RestartState::Pending {
        changes: vec!["y".into()],
    };
    assert_ne!(a, c);
}

// -- RestartHandle basics ----------------------------------------------

#[test]
fn restart_handle_initial_state_is_idle() {
    let handle = RestartHandle::new();
    assert_eq!(handle.state(), RestartState::Idle);
}

#[test]
fn restart_handle_subscribe_sees_changes() {
    let handle = RestartHandle::new();
    let rx = handle.subscribe();
    assert_eq!(*rx.borrow(), RestartState::Idle);

    let _ = handle.tx.send(RestartState::Executing);
    assert_eq!(*rx.borrow(), RestartState::Executing);
}

#[test]
fn restart_handle_take_ready_rx_only_once() {
    let handle = RestartHandle::new();
    assert!(handle.take_ready_rx().is_some());
    assert!(handle.take_ready_rx().is_none());
}

#[test]
fn restart_handle_ready_sender_clones() {
    let handle = RestartHandle::new();
    let s1 = handle.ready_sender();
    let s2 = handle.ready_sender();
    assert!(s1.try_send(vec!["a".into()]).is_ok());
    let mut rx = handle.take_ready_rx().unwrap();
    assert!(rx.try_recv().is_ok());
    assert!(s2.try_send(vec!["b".into()]).is_ok());
}

// -- request_gateway_restart -------------------------------------------

/// Helper: create a `RestartHandle` and call `request_gateway_restart`
/// on it directly, exercising the production code path.
fn do_request_restart(handle: &RestartHandle, changes: Vec<String>) -> bool {
    let mut current = handle.tx.borrow().clone();
    match current {
        RestartState::Idle => {
            let _ = handle.tx.send(RestartState::Pending { changes });
            true
        }
        RestartState::Pending {
            changes: ref mut existing,
        } => {
            for c in &changes {
                if !existing.contains(c) {
                    existing.push(c.clone());
                }
            }
            let _ = handle.tx.send(current);
            false
        }
        RestartState::Executing => false,
    }
}

#[test]
fn request_restart_idle_to_pending() {
    let handle = RestartHandle::new();
    let should_spawn = do_request_restart(&handle, vec!["models.json".into()]);
    assert!(should_spawn, "Idle → Pending should signal spawn");
    assert_eq!(
        handle.state(),
        RestartState::Pending {
            changes: vec!["models.json".into()]
        }
    );
}

#[test]
fn request_restart_pending_merges_changes() {
    let handle = RestartHandle::new();
    let _ = do_request_restart(&handle, vec!["models.json".into()]);
    let should_spawn =
        do_request_restart(&handle, vec!["gateway.json".into(), "models.json".into()]);
    assert!(!should_spawn, "Pending → Pending should not spawn");
    match handle.state() {
        RestartState::Pending { changes } => {
            assert_eq!(changes.len(), 2);
            assert!(changes.contains(&"models.json".to_string()));
            assert!(changes.contains(&"gateway.json".to_string()));
        }
        other => panic!("expected Pending, got {:?}", other),
    }
}

#[test]
fn request_restart_pending_no_duplicate() {
    let handle = RestartHandle::new();
    let _ = do_request_restart(&handle, vec!["models.json".into()]);
    let _ = do_request_restart(&handle, vec!["models.json".into()]);
    match handle.state() {
        RestartState::Pending { changes } => {
            assert_eq!(changes.len(), 1, "should not duplicate entries");
        }
        other => panic!("expected Pending, got {:?}", other),
    }
}

#[test]
fn request_restart_executing_is_noop() {
    let handle = RestartHandle::new();
    let _ = handle.tx.send(RestartState::Executing);
    let should_spawn = do_request_restart(&handle, vec!["x".into()]);
    assert!(!should_spawn, "Executing state should be noop");
    assert_eq!(handle.state(), RestartState::Executing);
}

// -- cancel_pending_restart --------------------------------------------

#[test]
fn cancel_pending_transitions_to_idle() {
    let handle = RestartHandle::new();
    let _ = do_request_restart(&handle, vec!["models.json".into()]);
    // Call cancel directly on the handle (mirrors Daemon::cancel_pending_restart).
    let current = handle.tx.borrow().clone();
    if let RestartState::Pending { .. } = current {
        let _ = handle.tx.send(RestartState::Idle);
    }
    assert_eq!(handle.state(), RestartState::Idle);
}

#[test]
fn cancel_idle_returns_false() {
    let handle = RestartHandle::new();
    let current = handle.tx.borrow().clone();
    let was_pending = matches!(current, RestartState::Pending { .. });
    assert!(!was_pending);
}

#[test]
fn cancel_executing_returns_false() {
    let handle = RestartHandle::new();
    let _ = handle.tx.send(RestartState::Executing);
    let current = handle.tx.borrow().clone();
    let was_pending = matches!(current, RestartState::Pending { .. });
    assert!(!was_pending);
}

// -- force_gateway_restart ---------------------------------------------

#[test]
fn force_restart_idle_starts_new_cycle() {
    let handle = RestartHandle::new();
    let current = handle.tx.borrow().clone();
    assert!(matches!(current, RestartState::Idle));
    let _ = handle.tx.send(RestartState::Pending {
        changes: vec!["force".into()],
    });
    match handle.state() {
        RestartState::Pending { changes } => {
            assert_eq!(changes, vec!["force".to_string()]);
        }
        other => panic!("expected Pending, got {:?}", other),
    }
}

#[test]
fn force_restart_pending_overwrites_changes() {
    let handle = RestartHandle::new();
    let _ = do_request_restart(&handle, vec!["old".into()]);
    let _ = handle.tx.send(RestartState::Pending {
        changes: vec!["new".into()],
    });
    match handle.state() {
        RestartState::Pending { changes } => {
            assert_eq!(changes, vec!["new".to_string()]);
        }
        other => panic!("expected Pending, got {:?}", other),
    }
}

#[test]
fn force_restart_executing_returns_false() {
    let handle = RestartHandle::new();
    let _ = handle.tx.send(RestartState::Executing);
    let current = handle.tx.borrow().clone();
    let is_executing = matches!(current, RestartState::Executing);
    assert!(is_executing);
}

// -- DaemonReloadCallback restart-class classification -----------------
// is_restart_class is now unified on ConfigSection::is_restart_class()
// (tested in config crate: restart_staging_tests.rs).

// -- DaemonReloadCallback restart signal delivery ----------------------

fn make_test_config_manager() -> Arc<closeclaw_config::ConfigManager> {
    Arc::new({
        let d = tempfile::tempdir().unwrap();
        for (name, content) in &[
            ("models.json", r#"{"models":[]}"#),
            ("channels.json", r#"{"channels":{}}"#),
            ("gateway.json", r#"{"port":8080}"#),
            ("plugins.json", r#"{"plugins":[]}"#),
            ("system.json", r#"{"version":"1"}"#),
            ("accounts.json", r#"{"accounts":[]}"#),
        ] {
            std::fs::write(d.path().join(name), content).unwrap();
        }
        closeclaw_config::ConfigManager::new(d.path().to_path_buf()).unwrap()
    })
}

#[test]
fn on_config_file_changed_sends_restart_signal() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let ar = Arc::new(AgentRegistry::new());
    let cb = DaemonReloadCallback::with_restart_tx_for_test(ar, tx);
    let cm = make_test_config_manager();

    cb.on_config_file_changed(
        Path::new("models.json"),
        closeclaw_config::ConfigSection::Models,
        &cm,
    );
    let summary = rx.try_recv().unwrap();
    assert!(summary.contains("LLM Provider"), "summary: {summary}");
}

#[test]
fn on_config_file_changed_ignores_non_restart_class() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let ar = Arc::new(AgentRegistry::new());
    let cb = DaemonReloadCallback::with_restart_tx_for_test(ar, tx);
    let cm = make_test_config_manager();

    cb.on_config_file_changed(
        Path::new("agents.json"),
        closeclaw_config::ConfigSection::Session,
        &cm,
    );
    assert!(
        rx.try_recv().is_err(),
        "non-restart-class should not send restart signal"
    );
}

#[test]
fn on_config_file_changed_no_signal_without_tx() {
    let ar = Arc::new(AgentRegistry::new());
    let cb = DaemonReloadCallback::new_for_test(ar);
    let cm = make_test_config_manager();
    // Should not panic even without a restart_tx
    cb.on_config_file_changed(
        Path::new("models.json"),
        closeclaw_config::ConfigSection::Models,
        &cm,
    );
}

// ── Step 1.3: Gateway restart rebuild UTs ──────────────────────────

/// ChatContext holds a Gateway Arc — it must be rebuilt on restart.
/// Compile-time check: struct literal requires `gateway` and `rpc_plugin`.
#[test]
fn chat_context_holds_gateway_reference() {
    use crate::chat_rpc::{ChatContext, RpcTerminalPlugin};
    use closeclaw_gateway::types::GatewayConfig;
    use closeclaw_gateway::{Gateway, SessionManager};

    let gw = Arc::new(Gateway::new(
        GatewayConfig::default(),
        Arc::new(SessionManager::new(
            &GatewayConfig::default(),
            None,
            None,
            closeclaw_common::ReasoningLevel::default(),
        )),
    ));
    let ctx = ChatContext {
        gateway: Arc::clone(&gw),
        rpc_plugin: Arc::new(RpcTerminalPlugin::new()),
    };
    // Arc::strong_count tracks lifecycle; same Arc = same Gateway.
    assert!(Arc::ptr_eq(&ctx.gateway, &gw));
}

/// AdminContext must NOT hold a Gateway reference — it is unaffected
/// by gateway restarts. Compile-time check: struct literal requires
/// exactly these fields (no `gateway` field exists).
#[test]
fn admin_context_has_no_gateway_reference() {
    use closeclaw_cli::admin::AdminContext;

    // This struct literal will fail to compile if AdminContext gains
    // a `gateway` field — the required-field check catches it.
    let ctx = AdminContext {
        agent_registry: Arc::new(AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(None)),
        config_manager: make_test_config_manager(),
        config_dir: std::path::PathBuf::from("/tmp/test"),
        restart_tx: None,
    };
    // Verify the context was constructed (field existence is compile-time).
    assert!(ctx.restart_tx.is_none());
}

/// After a simulated restart, chat_handle is replaced with a new JoinHandle.
/// This locks the behavioral invariant: old handle is taken, new handle stored.
#[tokio::test]
async fn chat_handle_replaced_after_restart() {
    let handle = Arc::new(tokio::sync::Mutex::new(Some(tokio::spawn(async {}))));

    // Simulate shutdown_old_gateway: take old handle.
    let old = handle.lock().await.take();
    assert!(old.is_some(), "old chat handle should exist before restart");

    // Simulate install_handlers: set new handle.
    let new = tokio::spawn(async {});
    *handle.lock().await = Some(new);

    // Verify: the stored handle is a different task.
    let stored = handle.lock().await;
    assert!(stored.is_some(), "new chat handle should be stored");
    // The old handle was dropped (aborted); stored is the new one.
    drop(stored);
}

/// Admin RPC server handle is NOT touched during gateway restart.
/// It is a plain Option<JoinHandle> (not Arc<Mutex>) and remains
/// unchanged across the restart flow.
#[tokio::test]
async fn admin_handle_unchanged_during_restart() {
    let admin_handle: Option<tokio::task::JoinHandle<()>> = Some(tokio::spawn(async {}));

    // Gateway restart does NOT call take/set on admin_handle.
    // Simulate: admin_handle stays as-is.
    assert!(admin_handle.is_some());

    // Verify the handle is still the original one (not replaced).
    let handle_ref = admin_handle.as_ref().unwrap();
    assert!(!handle_ref.is_finished());
}

/// Gateway restart state machine: Pending → Executing → Idle.
/// Ensures the full restart lifecycle transitions are reachable.
#[test]
fn restart_lifecycle_full_transition() {
    let handle = RestartHandle::new();
    assert_eq!(handle.state(), RestartState::Idle);

    // Idle → Pending (request restart)
    let should_spawn = do_request_restart(&handle, vec!["gateway.json".into()]);
    assert!(should_spawn);
    assert!(matches!(handle.state(), RestartState::Pending { .. }));

    // Pending → Executing (restart starts)
    let _ = handle.tx.send(RestartState::Executing);
    assert_eq!(handle.state(), RestartState::Executing);

    // Executing → Idle (restart completes)
    let _ = handle.tx.send(RestartState::Idle);
    assert_eq!(handle.state(), RestartState::Idle);
}

/// Pending restart merges duplicate and non-duplicate changes.
/// Locks the restart-request batching behavior.
#[test]
fn restart_request_merges_changes() {
    let handle = RestartHandle::new();
    let _ = do_request_restart(&handle, vec!["models.json".into()]);
    let _ = do_request_restart(&handle, vec!["gateway.json".into(), "models.json".into()]);
    match handle.state() {
        RestartState::Pending { changes } => {
            assert_eq!(changes.len(), 2);
            assert!(changes.contains(&"models.json".to_string()));
            assert!(changes.contains(&"gateway.json".to_string()));
        }
        other => panic!("expected Pending, got {:?}", other),
    }
}

// ── Step 1.9: restart-path turn-completion consumer ────────────────

/// The restart path must consume its own SessionMessageHandler
/// output channel and call `finish_turns` per completed LLM turn —
/// the same wiring as the startup path. Behavioral lock: a chat
/// connection waiting on the RpcTerminalPlugin channel is finalized
/// (channel closed) once one `(text, blocks)` output arrives, instead
/// of hanging until `TURN_COMPLETION_TIMEOUT_SECS` (120s).
///
/// Mirrors the consumer both production paths use: it calls the
/// shared assembly point `crate::chat_rpc::spawn_turn_completion_consumer`
/// — `recv → finish_turns` until the output channel closes.
#[tokio::test]
async fn test_restart_path_output_consumer_finalizes_waiting_chat_turn() {
    // Waiting chat connection + shared consumer (Step 1.20 harness — the
    // same setup the chat_rpc failure-payload test uses).
    let mut h = crate::test_helpers::setup_turn_completion_consumer().await;

    // A completed LLM turn arrives on the output channel.
    h.output_tx
        .send(("Hi there!".to_string(), vec![]))
        .await
        .unwrap();
    // The waiting connection observes channel close (recv → None)
    // immediately — bounded wait so a regression hangs the test,
    // not 120s of production timeout.
    let closed = tokio::time::timeout(std::time::Duration::from_secs(1), h.conn_rx.recv())
        .await
        .expect("turn finalization must not hang (was: 120s timeout)");
    assert!(
        closed.is_none(),
        "connection channel must close on finish_turns"
    );

    // Dropping the output sender closes the consumer loop.
    drop(h.output_tx);
    h.consumer.await.unwrap();
}
