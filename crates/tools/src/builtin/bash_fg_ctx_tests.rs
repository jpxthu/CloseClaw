//! Fg-chain exec-context consumption tests, split out of
//! `bash_tests.rs` to keep that file well below the 1000-line cap.
//!
//! Compiled via `#[path = "bash_fg_ctx_tests.rs"]` inside the bash
//! module, which grants access to its test-facing re-exports.

use super::tests::test_bg_pair;
use super::*;
use crate::builtin::bash_test_support::RecordingProgressSession;
use closeclaw_common::TaskManager;
use serde_json::json;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// fg-chain exec-context consumption: each foreground branch must consume
// the shared BashExecCtx fields (session / call_id / session_id /
// bg_manager) through the `exec` reference.
// ---------------------------------------------------------------------------

/// Completed branch: `ctx.exec.session` / `ctx.exec.call_id` must reach
/// `finalize_foreground_after_wait`, so the session receives progress
/// reports carrying the exact call_id.
#[tokio::test]
async fn test_handle_foreground_result_completed_reports_progress_via_ctx_session() {
    let (_bg_manager, bg_trait) = test_bg_pair();
    let (session, session_arc) = RecordingProgressSession::pair();
    let tmp = TempDir::new().unwrap();
    let child = spawn_sh_command("echo done", tmp.path().to_str().unwrap()).expect("spawn echo");
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));
    let exec_ctx = BashExecCtx {
        command: "echo done",
        cwd: tmp.path().to_str().unwrap(),
        bg_manager: &bg_trait,
        session: Some(&session_arc),
        call_id: Some("fg-complete-call"),
        session_id: "sess-complete",
    };
    let ctx = ForegroundContext {
        exec: &exec_ctx,
        manual_bg_signal: None,
        force_terminate: false,
    };
    let outcome = handle_foreground_result(
        child_arc,
        "echo done",
        std::time::Duration::from_secs(5),
        &ctx,
    )
    .await;

    let result = match outcome {
        ForegroundOutcome::Completed(r) => r,
        other => panic!("expected Completed, got: {:?}", other),
    };
    assert_eq!(result.data["exitCode"], json!(0));

    let calls = session.progress_calls();
    assert!(
        !calls.is_empty(),
        "completed path must report progress through ctx.exec.session"
    );
    assert!(
        calls.iter().all(|(cid, _)| cid == "fg-complete-call"),
        "progress reports must carry ctx.exec.call_id unchanged, got: {:?}",
        calls.iter().map(|(cid, _)| cid).collect::<Vec<_>>()
    );
}

/// Force-terminate branch: the kill-drain must pass `ctx.exec.session` /
/// `ctx.exec.call_id` to `read_with_progress`, so the session receives a
/// progress report carrying the exact call_id.
#[tokio::test]
async fn test_handle_timeout_expiry_force_terminate_reports_progress_via_ctx_session() {
    let (_bg_manager, bg_trait) = test_bg_pair();
    let (session, session_arc) = RecordingProgressSession::pair();
    let tmp = TempDir::new().unwrap();
    let mut child = spawn_sh_command("sleep 3", tmp.path().to_str().unwrap()).expect("spawn sleep");
    let handles = ChildHandles {
        stdout_handle: child.stdout.take(),
        stderr_handle: child.stderr.take(),
        child,
    };
    let exec_ctx = BashExecCtx {
        command: "sleep 3",
        cwd: tmp.path().to_str().unwrap(),
        bg_manager: &bg_trait,
        session: Some(&session_arc),
        call_id: Some("fg-force-kill-call"),
        session_id: "sess-force-kill",
    };
    let ctx = ForegroundContext {
        exec: &exec_ctx,
        manual_bg_signal: None,
        force_terminate: true,
    };
    let outcome = handle_timeout_expiry(handles, "sleep 3", &ctx).await;

    match outcome {
        ForegroundOutcome::Failed(msg) => {
            assert!(
                msg.contains("exceeded total execution time limit"),
                "expected force-kill message, got: {}",
                msg
            );
        }
        other => panic!("expected Failed (force-terminated), got: {:?}", other),
    }

    let calls = session.progress_calls();
    assert!(
        !calls.is_empty(),
        "force-terminate drain must report progress through ctx.exec.session"
    );
    assert!(
        calls.iter().all(|(cid, _)| cid == "fg-force-kill-call"),
        "drain progress reports must carry ctx.exec.call_id unchanged, got: {:?}",
        calls.iter().map(|(cid, _)| cid).collect::<Vec<_>>()
    );
}

/// Manual-signal branch: `ctx.exec.session_id` must be forwarded
/// unchanged to the backgroundize registration (mirrors the timeout
/// branch assertion in backgroundize tests).
#[tokio::test]
async fn test_handle_foreground_result_manual_bg_forwards_ctx_session_id() {
    let fake = FakeTaskManager::new().with_pipe_assertions();
    let bg_trait: Arc<dyn TaskManager> = Arc::new(fake.clone());
    let tmp = TempDir::new().unwrap();
    let child = spawn_sh_command("sleep 5", tmp.path().to_str().unwrap()).expect("spawn sleep");
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));
    let signal = Arc::new(tokio::sync::Notify::new());
    let signal_clone = Arc::clone(&signal);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        signal_clone.notify_waiters();
    });
    let exec_ctx = BashExecCtx {
        command: "sleep 5",
        cwd: tmp.path().to_str().unwrap(),
        bg_manager: &bg_trait,
        session: None,
        call_id: None,
        session_id: "sess-manual-bg",
    };
    let ctx = ForegroundContext {
        exec: &exec_ctx,
        manual_bg_signal: Some(&signal),
        force_terminate: false,
    };
    let outcome = handle_foreground_result(
        child_arc,
        "sleep 5",
        std::time::Duration::from_secs(5),
        &ctx,
    )
    .await;

    let result = match outcome {
        ForegroundOutcome::AutoBackground(r, _) => r,
        other => panic!("expected AutoBackground (manual bg), got: {:?}", other),
    };
    assert_eq!(
        result.data["backgroundedByUser"],
        json!(true),
        "manual-signal branch must build the manual result shape"
    );
    assert!(
        result.data["assistantAutoBackgrounded"].is_null(),
        "manual-signal branch must not set assistantAutoBackgrounded"
    );

    let calls = fake.backgroundize_calls();
    assert_eq!(calls.len(), 1, "exactly one backgroundize_task call");
    assert!(
        calls[0].is_backgrounded,
        "manual-signal branch must register is_backgrounded=true"
    );
    assert_eq!(
        calls[0].session_id, "sess-manual-bg",
        "manual-signal branch must forward ctx.exec.session_id unchanged"
    );

    let task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("backgroundTaskId must be present")
        .to_string();
    let _ = bg_trait.kill_task(&task_id).await;
}
