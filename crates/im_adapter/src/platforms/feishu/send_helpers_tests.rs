//! Convention (see `test_helpers.rs` module docs): any test that creates
//! and actually spawns a mock CLI script must be marked
//! `#[serial_test::serial]` (ETXTBSY root cause); mock scripts must live
//! in a `tempfile::TempDir`.

use super::send_helpers::{
    build_reply_args, build_send_args, is_capability_error, make_media_path_relative, run_cli,
    ReplyTarget,
};
use super::{create_failing_mock_cli, create_mock_cli, make_adapter, write_mock_script};
use crate::error::AdapterError;
use serial_test::serial;
use std::fmt::Write as _;
use std::sync::Arc;
use tempfile::TempDir;

#[serial]
#[tokio::test]
async fn test_run_cli_success() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, r#"{"code":0,"msg":"ok"}"#);
    let adapter = make_adapter(&cli);
    let result = run_cli(&adapter, &["im", "+messages-send"]).await;
    assert!(result.is_ok());
}

#[serial]
#[tokio::test]
async fn test_run_cli_error_code() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, r#"{"code":999,"msg":"something failed"}"#);
    let adapter = make_adapter(&cli);
    let result = run_cli(&adapter, &["im", "+messages-send"]).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        AdapterError::SendFailed(msg) => {
            assert!(msg.contains("999"));
            assert!(msg.contains("something failed"));
        }
        other => panic!("expected SendFailed, got {:?}", other),
    }
}

#[serial]
#[tokio::test]
async fn test_run_cli_non_json_output() {
    let tmp = TempDir::new().unwrap();
    let cli = create_mock_cli(&tmp, "ok");
    let adapter = make_adapter(&cli);
    let result = run_cli(&adapter, &["--version"]).await;
    assert!(result.is_ok());
}

#[serial]
#[tokio::test]
async fn test_run_cli_process_failure() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(
        &tmp,
        "failing_lark_cli.sh",
        r#"{"code":1,"msg":"error"}"#,
        false,
        1,
    );
    let adapter = make_adapter(&cli);
    let result = run_cli(&adapter, &["im", "+messages-send"]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_run_cli_command_not_found() {
    let adapter = make_adapter("nonexistent_command_xyz");
    let result = run_cli(&adapter, &["--version"]).await;
    assert!(result.is_err());
}

#[test]
fn test_build_send_args_text() {
    let args = build_send_args("oc_chat", "text", "hello");
    assert_eq!(args[0], "im");
    assert_eq!(args[1], "+messages-send");
    assert!(args.contains(&"--chat-id".to_string()));
    assert!(args.contains(&"--text".to_string()));
    assert!(args.contains(&"hello".to_string()));
    assert!(args.contains(&"--as".to_string()));
    assert!(args.contains(&"bot".to_string()));
}

#[test]
fn test_build_send_args_interactive() {
    let args = build_send_args("oc_chat", "interactive", "{}");
    assert!(args.contains(&"--msg-type".to_string()));
    assert!(args.contains(&"interactive".to_string()));
    assert!(args.contains(&"--content".to_string()));
}

#[test]
fn test_build_reply_args_thread() {
    let target = ReplyTarget::Thread {
        root_id: "om_msg123".to_string(),
    };
    let args = build_reply_args(&target, "reply text");
    assert_eq!(args[0], "im");
    assert_eq!(args[1], "+messages-reply");
    assert!(args.contains(&"--message-id".to_string()));
    assert!(args.contains(&"om_msg123".to_string()));
    assert!(args.contains(&"--reply-in-thread".to_string()));
}

#[test]
fn test_build_reply_args_message() {
    let target = ReplyTarget::Message {
        message_id: "om_msg456".to_string(),
    };
    let args = build_reply_args(&target, "reply text");
    assert_eq!(args[0], "im");
    assert_eq!(args[1], "+messages-reply");
    assert!(args.contains(&"--message-id".to_string()));
    assert!(args.contains(&"om_msg456".to_string()));
    assert!(!args.contains(&"--reply-in-thread".to_string()));
}

#[test]
fn test_is_capability_error() {
    assert!(is_capability_error(
        "lark-cli error 230001: capability not supported"
    ));
    assert!(is_capability_error("lark-cli error 230002: unsupported"));
    assert!(!is_capability_error("lark-cli error 200: ok"));
    assert!(!is_capability_error("lark-cli error 0: success"));
    assert!(!is_capability_error("lark-cli error 99999: unknown"));
    assert!(!is_capability_error("some other error"));
}

/// Create a mock lark-cli script that echoes all arguments as JSON.
fn create_args_echo_cli(tmp: &TempDir) -> String {
    let script_path = tmp.path().join("echo_args.sh");
    let args_file = tmp.path().join("captured_args");
    let args_path = args_file.to_str().unwrap();
    // Script that writes all args to a file, one per line
    let mut content = String::new();
    writeln!(content, "#!/bin/bash").unwrap();
    writeln!(content, "echo \"$@\" > {args_path}").unwrap();
    writeln!(content, "echo '{{\"code\":0}}'").unwrap();
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}

#[serial]
#[tokio::test]
async fn test_run_cli_inserts_profile_arg() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let _ = run_cli(&adapter, &["im", "+messages-send"]).await;
    // Verify --profile was prepended to the args
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(
        content.contains("--profile"),
        "Expected --profile in args, got: {content}"
    );
    assert!(
        content.contains("test_profile"),
        "Expected profile name in args, got: {content}"
    );
    // Verify the original args are still present
    assert!(content.contains("im"));
    assert!(content.contains("+messages-send"));
}

// =====================================================================
// send_msg composite peer_id tests
// =====================================================================

/// Composite peer_id without reply_ref → extract user-id, send new message.
#[serial]
#[tokio::test]
async fn test_send_msg_composite_peer_id_no_reply() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let result = adapter
        .send_msg("ou_user123|om_msg456", "text", "hello", None)
        .await;
    assert!(result.is_ok());
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(content.contains("--user-id"));
    assert!(content.contains("ou_user123"));
    assert!(!content.contains("om_msg456"));
    assert!(content.contains("+messages-send"));
}

/// Composite peer_id with Thread reply_ref → use +messages-reply --reply-in-thread.
#[serial]
#[tokio::test]
async fn test_send_msg_composite_peer_id_thread_reply() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let target = ReplyTarget::Thread {
        root_id: "om_root789".to_string(),
    };
    let result = adapter
        .send_msg("ou_user123|om_msg456", "text", "reply", Some(&target))
        .await;
    assert!(result.is_ok());
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(content.contains("+messages-reply"));
    assert!(content.contains("--message-id"));
    assert!(content.contains("om_root789"));
    assert!(content.contains("--reply-in-thread"));
}

/// Composite peer_id with Message reply_ref → use +messages-reply without --reply-in-thread.
#[serial]
#[tokio::test]
async fn test_send_msg_composite_peer_id_message_reply() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let target = ReplyTarget::Message {
        message_id: "om_msg456".to_string(),
    };
    let result = adapter
        .send_msg("ou_user123|om_msg456", "text", "reply", Some(&target))
        .await;
    assert!(result.is_ok());
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(content.contains("+messages-reply"));
    assert!(content.contains("--message-id"));
    assert!(content.contains("om_msg456"));
    assert!(!content.contains("--reply-in-thread"));
}

/// Simple (non-composite) peer_id without reply_ref → send new message.
#[serial]
#[tokio::test]
async fn test_send_msg_simple_peer_id_no_reply() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let result = adapter.send_msg("oc_chat123", "text", "hello", None).await;
    assert!(result.is_ok());
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(content.contains("--chat-id"));
    assert!(content.contains("oc_chat123"));
    assert!(content.contains("+messages-send"));
}

/// Composite peer_id with group chat (oc_) → extract user-id portion.
#[serial]
#[tokio::test]
async fn test_send_msg_composite_peer_id_group_chat() {
    let tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&tmp);
    let adapter = make_adapter(&cli);
    let result = adapter
        .send_msg("ou_user123|oc_chat456", "text", "hello", None)
        .await;
    assert!(result.is_ok());
    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    // Should use --user-id with the extracted user portion
    assert!(content.contains("--user-id"));
    assert!(content.contains("ou_user123"));
    assert!(!content.contains("oc_chat456"));
}

// =====================================================================
// send_media_file path conversion tests
// =====================================================================

/// When outbound_path is under cwd, lark-cli receives a relative path.
///
/// Places a source file in the media store's storage dir (which is under cwd),
/// so `prepare_outbound_local_media` copies it to outbound/ and `send_media_file`
/// converts the outbound path to relative.
#[serial]
#[tokio::test]
async fn test_send_media_file_outbound_under_cwd_uses_relative_path() {
    // Create temp dir under the project cwd so outbound path can be stripped.
    let cwd = std::env::current_dir().unwrap();
    let tmp = TempDir::new_in(&cwd).unwrap();
    let cli = create_args_echo_cli(&tmp);

    // Place media store under tmp (which is under cwd).
    let media_dir = tmp.path().join("media");
    let media_store = crate::media_store::MediaStore::new(media_dir.to_str().unwrap()).unwrap();
    // Source file must be inside an allowed dir (media store storage dir).
    let src = media_dir.join("photo.png");
    std::fs::write(&src, "image data").unwrap();

    let mut adapter = make_adapter(&cli);
    adapter.media_store = Arc::new(media_store);

    let result = adapter.send_image("oc_test", src.to_str().unwrap()).await;
    assert!(result.is_ok());

    let args_file = tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    let image_arg = content
        .split_whitespace()
        .skip_while(|s| *s != "--image")
        .nth(1)
        .expect("--image arg not found");
    assert!(
        !std::path::Path::new(image_arg).is_absolute(),
        "Expected relative path under cwd, got absolute: {image_arg}"
    );
    assert!(
        image_arg.contains("outbound/"),
        "Expected outbound in path: {image_arg}"
    );
    // Verify the file actually exists at that relative path from cwd.
    assert!(std::path::Path::new(image_arg).exists());
}

/// When outbound_path is not under cwd, lark-cli receives an absolute path.
///
/// Places media store under the system temp dir (outside project cwd), so the
/// outbound path cannot be stripped to a cwd-relative path.
#[serial]
#[tokio::test]
async fn test_send_media_file_outbound_outside_cwd_uses_absolute_path() {
    let external_dir = TempDir::new().unwrap();
    let media_dir = external_dir.path().join("media");
    let media_store = crate::media_store::MediaStore::new(media_dir.to_str().unwrap()).unwrap();
    let src = media_dir.join("photo.png");
    std::fs::write(&src, "image data").unwrap();

    let cli_tmp = TempDir::new().unwrap();
    let cli = create_args_echo_cli(&cli_tmp);
    let mut adapter = make_adapter(&cli);
    adapter.media_store = Arc::new(media_store);

    let result = adapter.send_image("oc_test", src.to_str().unwrap()).await;
    assert!(result.is_ok());

    let args_file = cli_tmp.path().join("captured_args");
    let content = std::fs::read_to_string(&args_file).unwrap_or_default();
    let image_arg = content
        .split_whitespace()
        .skip_while(|s| *s != "--image")
        .nth(1)
        .expect("--image arg not found");
    assert!(
        std::path::Path::new(image_arg).is_absolute(),
        "Expected absolute path for outbound outside cwd, got: {image_arg}"
    );
    assert!(std::path::Path::new(image_arg).exists());
}

/// Unit test for make_media_path_relative: outbound is not under cwd,
/// so the function preserves the absolute path.
#[test]
fn test_make_media_path_relative_outbound_not_under_cwd() {
    let cwd = std::path::PathBuf::from("/some/other/directory");
    let outbound = std::path::PathBuf::from("/some/unreachable/path/outbound/photo.png");
    let result = make_media_path_relative(&outbound, &cwd);
    assert_eq!(result, outbound.to_string_lossy());
}

/// Unit test for make_media_path_relative: outbound under cwd → relative.
#[test]
fn test_make_media_path_relative_under_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let outbound = cwd.join("workspace").join("outbound").join("photo.png");
    let result = make_media_path_relative(&outbound, &cwd);
    assert_eq!(result, "workspace/outbound/photo.png");
}

/// Unit test for make_media_path_relative: outbound outside cwd → absolute.
#[test]
fn test_make_media_path_relative_outside_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let outbound = std::path::PathBuf::from("/completely/different/dir/photo.png");
    let result = make_media_path_relative(&outbound, &cwd);
    assert_eq!(result, outbound.to_string_lossy());
}

/// Unit test for make_media_path_relative: outbound == cwd (empty strip_prefix)
/// should preserve absolute path instead of returning empty string.
#[test]
fn test_make_media_path_relative_empty_relative_path_preserves_absolute() {
    let cwd = std::path::PathBuf::from("/some/cwd/dir");
    let result = make_media_path_relative(&cwd, &cwd);
    assert_eq!(result, cwd.to_string_lossy());
}
