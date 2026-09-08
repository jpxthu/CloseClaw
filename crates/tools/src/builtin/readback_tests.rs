//! Unit tests for the readback module.
//!
//! Tests cover the recovery path (write fails but readback matches),
//! the non-recovery path, missing files, and the decision function.

use crate::builtin::readback;
use closeclaw_common::Tool;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_root() -> bool {
    // Read /proc/self/status to get Uid without depending on libc.
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:\t") {
            if let Some(uid_str) = rest.split('\t').next() {
                return uid_str == "0";
            }
        }
    }
    false
}

fn make_readonly(path: &std::path::Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
}

fn restore_writable(path: &std::path::Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

// ---------------------------------------------------------------------------
// Decision function — is_recovered
// ---------------------------------------------------------------------------

#[test]
fn is_recovered_returns_true_when_content_matches() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("match.txt");
    std::fs::write(&path, "expected content").unwrap();
    assert!(readback::is_recovered(&path, "expected content"));
}

#[test]
fn is_recovered_returns_false_when_content_mismatches() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("mismatch.txt");
    std::fs::write(&path, "actual content").unwrap();
    assert!(!readback::is_recovered(&path, "expected content"));
}

#[test]
fn is_recovered_returns_false_for_missing_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("does_not_exist.txt");
    assert!(!readback::is_recovered(&path, "anything"));
}

// ---------------------------------------------------------------------------
// Core behavior — write_with_readback recovery path
// ---------------------------------------------------------------------------

/// When `std::fs::write` fails (e.g. EACCES on a read-only file) but the
/// file already contains the expected content, `write_with_readback` should
/// return Ok — treating the write as successful.
#[test]
fn write_with_readback_recovers_when_content_matches() {
    if is_root() {
        eprintln!("SKIP: root ignores permission bits");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("recover.txt");

    // Write expected content first, then make read-only.
    std::fs::write(&path, "hello world").unwrap();
    make_readonly(&path);

    // fs::write will fail (EACCES), but readback should recover.
    let result = readback::write_with_readback(&path, "hello world");
    assert!(result.is_ok(), "readback should recover on content match");

    // Cleanup: restore permissions so TempDir can clean up.
    restore_writable(&path);
}

// ---------------------------------------------------------------------------
// Non-recovery path — content mismatch
// ---------------------------------------------------------------------------

/// When `std::fs::write` fails and the file content does not match the
/// expected content, the original error must be propagated.
#[test]
fn write_with_readback_does_not_recover_when_content_mismatches() {
    if is_root() {
        eprintln!("SKIP: root ignores permission bits");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("no_recover.txt");

    // Write different content, then make read-only.
    std::fs::write(&path, "old content").unwrap();
    make_readonly(&path);

    // fs::write will fail, and readback won't match "new content".
    let result = readback::write_with_readback(&path, "new content");
    assert!(result.is_err(), "should fail when content doesn't match");

    // Verify the error contains the original write error info.
    match result.unwrap_err() {
        crate::ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("Permission denied") || msg.contains("os error 13"),
                "error should contain OS error details: {msg}"
            );
        }
        other => panic!("expected ExecutionFailed, got: {other:?}"),
    }

    // Cleanup.
    restore_writable(&path);
}

// ---------------------------------------------------------------------------
// Write succeeds — no readback needed
// ---------------------------------------------------------------------------

/// When `std::fs::write` succeeds, readback is never invoked and Ok is
/// returned directly.
#[test]
fn write_with_readback_ok_when_write_succeeds() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("normal.txt");

    let result = readback::write_with_readback(&path, "written content");
    assert!(result.is_ok());

    let actual = std::fs::read_to_string(&path).unwrap();
    assert_eq!(actual, "written content");
}

// ---------------------------------------------------------------------------
// Tool-level integration — WriteTool
// ---------------------------------------------------------------------------

/// WriteTool writing to a read-only file that already contains expected
/// content should succeed via readback recovery.
#[tokio::test]
async fn write_tool_recovers_on_readonly_file_with_matching_content() {
    use crate::builtin::file_ops::tests::{
        allow_file, allow_tool, make_af, make_cm, make_ctx, make_engine, make_sm,
    };
    use crate::builtin::file_ops::WriteTool;

    if is_root() {
        eprintln!("SKIP: root ignores permission bits");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("tool_recover.txt");

    // Pre-write expected content, then lock down.
    std::fs::write(&path, "expected").unwrap();
    make_readonly(&path);

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = WriteTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "expected"
    });
    let result: Result<crate::ToolResult, crate::ToolCallError> =
        tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok(), "WriteTool should recover via readback");

    let tool_result = result.unwrap();
    // Data shape must match normal success: {"content": "..."}.
    assert_eq!(tool_result.data["content"], "expected");

    // Cleanup.
    restore_writable(&path);
}

// ---------------------------------------------------------------------------
// Tool-level integration — EditTool
// ---------------------------------------------------------------------------

/// EditTool applies edits through the readback-integrated path.
/// The readback recovery mechanism is validated by `write_with_readback`
/// unit tests; this verifies the full EditTool pipeline works end-to-end.
#[tokio::test]
async fn edit_tool_succeeds_through_readback_path() {
    use crate::builtin::file_ops::tests::{
        allow_file, allow_tool, make_af, make_cm, make_ctx, make_engine, make_sm,
    };
    use crate::builtin::file_ops::EditTool;

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("tool_edit_readback.txt");
    std::fs::write(&path, "old text here").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "oldText": "old text",
        "newText": "new text"
    });
    let result: Result<crate::ToolResult, crate::ToolCallError> =
        tool.call(args, &make_ctx("a")).await;
    assert!(
        result.is_ok(),
        "EditTool should succeed through readback path"
    );

    let tool_result = result.unwrap();
    // Data shape must match normal success: {"content": "..."}.
    assert_eq!(tool_result.data["content"], "new text here");
    let actual = std::fs::read_to_string(&path).unwrap();
    assert_eq!(actual, "new text here");
}
