//! Feishu adapter send helpers — lark-cli subprocess execution.
//!
//! All message sending goes through `lark-cli` subprocess commands:
//! - Text: `lark-cli im +messages-send --chat-id <id> --text "..." --as bot`
//! - Card: `lark-cli im +messages-send --chat-id <id> --msg-type interactive --content '<json>' --as bot`
//! - Reply: `lark-cli im +messages-reply --message-id <id> --text "..." --reply-in-thread`
//! - Media: `lark-cli im +messages-send --chat-id <id> --image <path>` or `--file <path>`
//!
//! HTTP token management and direct API calls are no longer used for sending.

use super::adapter::FeishuAdapter;
use crate::error::AdapterError;

use tokio::process::Command;

/// Execute a lark-cli command and return the result.
///
/// Spawns the subprocess, captures stdout/stderr, and parses the JSON
/// response. Automatically prepends `["--profile", &adapter.profile]`
/// to ensure all lark-cli commands use the correct credential profile.
/// Returns `Ok(stdout)` on successful execution, or
/// `Err(SendFailed)` if the process fails to start, exits with error,
/// or returns a non-zero `code` in its JSON output.
pub(crate) async fn run_cli(
    adapter: &FeishuAdapter,
    args: &[&str],
) -> Result<String, AdapterError> {
    let output = spawn_cli(adapter, args).await?;
    check_cli_exit_status(&output)?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    check_cli_json_error(&stdout)?;
    Ok(stdout)
}

/// Spawn a lark-cli subprocess with --profile prepended.
async fn spawn_cli(
    adapter: &FeishuAdapter,
    args: &[&str],
) -> Result<std::process::Output, AdapterError> {
    let mut full_args: Vec<&str> = vec!["--profile", &adapter.profile];
    full_args.extend_from_slice(args);
    Command::new(&adapter.cli_command)
        .args(&full_args)
        .output()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, command = %adapter.cli_command, "failed to spawn lark-cli");
            AdapterError::SendFailed(format!("lark-cli spawn error: {e}"))
        })
}

/// Check if the process exited with a non-zero status.
fn check_cli_exit_status(output: &std::process::Output) -> Result<(), AdapterError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    tracing::warn!(
        code = ?output.status.code(),
        stderr = %stderr,
        "lark-cli exited with error"
    );
    Err(AdapterError::SendFailed(format!(
        "lark-cli exited with code {:?}: {}",
        output.status.code(),
        stderr.trim()
    )))
}

/// Check if a JSON response contains an error code.
fn check_cli_json_error(stdout: &str) -> Result<(), AdapterError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let code = val.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        if code != 0 {
            let msg = val
                .get("msg")
                .or_else(|| val.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            tracing::warn!(code = code, msg = %msg, "lark-cli returned error");
            return Err(AdapterError::SendFailed(format!(
                "lark-cli error {code}: {msg}"
            )));
        }
    }
    Ok(())
}

/// Target type for outgoing reply routing.
///
/// Determines which `lark-cli` command and flags to use:
/// - `Thread`: `+messages-reply --message-id <root_id> --reply-in-thread`
/// - `Message`: `+messages-reply --message-id <message_id>` (no thread flag)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyTarget {
    /// Reply to a thread root (creates or continues a thread).
    Thread { root_id: String },
    /// Reply to a specific message (direct reply, not in thread).
    /// Used when Gateway passes top-level peer_id as a reply target
    /// (design doc "会话锚点构造").
    Message { message_id: String },
}

/// Build lark-cli arguments for sending a message.
///
/// Returns the argument list (without the command name) for:
/// - P2P (open_id `ou_xxx`): `lark-cli im +messages-send --user-id <id> --<flag> <content> --as bot`
/// - Group (chat_id `oc_xxx`): `lark-cli im +messages-send --chat-id <id> --<flag> <content> --as bot`
fn build_send_args(chat_id: &str, msg_type: &str, content: &str) -> Vec<String> {
    let (id_flag, id_value) = if chat_id.starts_with("ou_") {
        ("--user-id".to_string(), chat_id.to_string())
    } else {
        ("--chat-id".to_string(), chat_id.to_string())
    };
    let mut args = vec![
        "im".to_string(),
        "+messages-send".to_string(),
        id_flag,
        id_value,
    ];

    match msg_type {
        "text" => {
            args.push("--text".to_string());
            args.push(content.to_string());
        }
        "interactive" => {
            args.push("--msg-type".to_string());
            args.push("interactive".to_string());
            args.push("--content".to_string());
            args.push(content.to_string());
        }
        other => {
            tracing::warn!(msg_type = other, "unknown msg_type, sending as text");
            args.push("--text".to_string());
            args.push(content.to_string());
        }
    }

    args.push("--as".to_string());
    args.push("bot".to_string());
    args
}

/// Build lark-cli arguments for replying to a message.
///
/// `ReplyTarget::Thread` → `+messages-reply --message-id <root_id> --text "..." --reply-in-thread`
/// `ReplyTarget::Message` → `+messages-reply --message-id <id> --text "..."`
fn build_reply_args(target: &ReplyTarget, text: &str) -> Vec<String> {
    let mut args = vec![
        "im".to_string(),
        "+messages-reply".to_string(),
        "--message-id".to_string(),
    ];
    match target {
        ReplyTarget::Thread { root_id } => {
            args.push(root_id.clone());
            args.push("--text".to_string());
            args.push(text.to_string());
            args.push("--reply-in-thread".to_string());
        }
        ReplyTarget::Message { message_id } => {
            args.push(message_id.clone());
            args.push("--text".to_string());
            args.push(text.to_string());
        }
    }
    args
}

impl FeishuAdapter {
    /// Low-level: send a message via lark-cli subprocess.
    ///
    /// Routes to `+messages-reply` when `reply_ref` is `Some` (thread or
    /// message reply), otherwise to `+messages-send`.
    ///
    /// When `receive_id` is a composite peer_id (`{user_id}|{rest}`),
    /// the user identifier portion (before the first `|`) is extracted
    /// for the `--user-id` flag.
    pub(crate) async fn send_msg(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &str,
        reply_ref: Option<&ReplyTarget>,
    ) -> Result<(), AdapterError> {
        // Extract user identifier from composite peer_id.
        let effective_id = match receive_id.find('|') {
            Some(pos) => &receive_id[..pos],
            None => receive_id,
        };
        let args = match reply_ref {
            Some(target) => build_reply_args(target, content),
            None => build_send_args(effective_id, msg_type, content),
        };
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_cli(self, &arg_refs).await?;
        Ok(())
    }

    /// Attempt to send the card's text content as a plain text message.
    ///
    /// Used when `send_card_json` fails with a capability error
    /// (e.g. unsupported `select_static` component). Extracts
    /// markdown/plain_text content from the card payload via
    /// `renderer::extract_card_plain_text` and sends it through the
    /// text message API.
    pub(crate) async fn try_fallback_to_text(
        &self,
        chat_id: &str,
        card_json: &str,
        reply_ref: Option<&ReplyTarget>,
    ) -> Result<(), AdapterError> {
        let card_value: serde_json::Value =
            serde_json::from_str(card_json).unwrap_or(serde_json::Value::Null);
        let plain_text = super::renderer::extract_card_plain_text(&card_value);
        if plain_text.is_empty() {
            tracing::warn!(
                receive_id = %chat_id,
                "Capability fallback: no extractable text in card"
            );
            return Ok(());
        }
        self.send_msg(chat_id, "text", &plain_text, reply_ref).await
    }

    /// Send an image file via lark-cli.
    ///
    /// - P2P (`ou_xxx`): `lark-cli im +messages-send --user-id <id> --image <path> --as bot`
    /// - Group (`oc_xxx`): `lark-cli im +messages-send --chat-id <id> --image <path> --as bot`
    pub(crate) async fn send_image(
        &self,
        receive_id: &str,
        image_path: &str,
    ) -> Result<(), AdapterError> {
        self.send_media_file(receive_id, "--image", image_path)
            .await
    }

    /// Send a file via lark-cli.
    ///
    /// - P2P (`ou_xxx`): `lark-cli im +messages-send --user-id <id> --file <path> --as bot`
    /// - Group (`oc_xxx`): `lark-cli im +messages-send --chat-id <id> --file <path> --as bot`
    pub(crate) async fn send_file(
        &self,
        receive_id: &str,
        file_path: &str,
    ) -> Result<(), AdapterError> {
        self.send_media_file(receive_id, "--file", file_path).await
    }

    /// Send a media file (image or file) via lark-cli.
    ///
    /// Common implementation for `send_image` and `send_file`.
    /// Routes to `--user-id` for P2P (`ou_xxx`) or `--chat-id` for group chats (`oc_xxx`).
    async fn send_media_file(
        &self,
        receive_id: &str,
        media_flag: &str,
        file_path: &str,
    ) -> Result<(), AdapterError> {
        let id_flag = if receive_id.starts_with("ou_") {
            "--user-id"
        } else {
            "--chat-id"
        };
        let args = vec![
            "im",
            "+messages-send",
            id_flag,
            receive_id,
            media_flag,
            file_path,
            "--as",
            "bot",
        ];
        run_cli(self, &args).await?;
        Ok(())
    }

    /// Send an emoji reaction to a message via lark-cli.
    ///
    /// `lark-cli im reactions create --params '{"message_id":"<id>"}' --data '{"reaction_type":{"emoji_type":"<emoji>"}}'`
    #[allow(dead_code)]
    pub(crate) async fn send_reaction(
        &self,
        message_id: &str,
        emoji_type: &str,
    ) -> Result<(), AdapterError> {
        let params = serde_json::json!({"message_id": message_id}).to_string();
        let data = serde_json::json!({
            "reaction_type": {"emoji_type": emoji_type}
        })
        .to_string();
        let args = vec![
            "im",
            "reactions",
            "create",
            "--params",
            &params,
            "--data",
            &data,
        ];
        run_cli(self, &args).await?;
        Ok(())
    }
}

/// Check whether an error message is a lark-cli "capability" error.
///
/// These are non-fatal errors where a text fallback is appropriate.
/// For lark-cli, capability errors come through as error codes 230001 or
/// 230002 in the JSON response.
pub(crate) fn is_capability_error(error_msg: &str) -> bool {
    error_msg.contains("230001") || error_msg.contains("230002")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Create a mock lark-cli script that outputs the given JSON and exits 0.
    fn create_mock_cli(tmp: &TempDir, response_json: &str) -> String {
        let script_path = tmp.path().join("mock_lark_cli.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/bash").unwrap();
        writeln!(f, "echo '{response_json}'").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }
        script_path.to_str().unwrap().to_string()
    }

    /// Create a mock lark-cli script that exits with a non-zero code.
    fn create_failing_mock_cli(tmp: &TempDir, exit_code: i32) -> String {
        let script_path = tmp.path().join("failing_lark_cli.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/bash").unwrap();
        writeln!(f, "echo '{{\"code\":{exit_code},\"msg\":\"error\"}}' >&2").unwrap();
        writeln!(f, "exit 1").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }
        script_path.to_str().unwrap().to_string()
    }

    fn make_adapter_with_cli(cli_command: &str) -> FeishuAdapter {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut adapter = FeishuAdapter::new(
            "test_profile".into(),
            Arc::new(crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).unwrap()),
        );
        adapter.cli_command = cli_command.to_string();
        adapter
    }

    #[tokio::test]
    async fn test_run_cli_success() {
        let tmp = TempDir::new().unwrap();
        let cli = create_mock_cli(&tmp, r#"{"code":0,"msg":"ok"}"#);
        let adapter = make_adapter_with_cli(&cli);
        let result = run_cli(&adapter, &["im", "+messages-send"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_cli_error_code() {
        let tmp = TempDir::new().unwrap();
        let cli = create_mock_cli(&tmp, r#"{"code":999,"msg":"something failed"}"#);
        let adapter = make_adapter_with_cli(&cli);
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

    #[tokio::test]
    async fn test_run_cli_non_json_output() {
        let tmp = TempDir::new().unwrap();
        let cli = create_mock_cli(&tmp, "ok");
        let adapter = make_adapter_with_cli(&cli);
        let result = run_cli(&adapter, &["--version"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_cli_process_failure() {
        let tmp = TempDir::new().unwrap();
        let cli = create_failing_mock_cli(&tmp, 1);
        let adapter = make_adapter_with_cli(&cli);
        let result = run_cli(&adapter, &["im", "+messages-send"]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_cli_command_not_found() {
        let adapter = make_adapter_with_cli("nonexistent_command_xyz");
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
        let mut f = std::fs::File::create(&script_path).unwrap();
        // Script that writes all args to a file, one per line
        writeln!(f, "#!/bin/bash").unwrap();
        writeln!(f, "echo \"$@\" > {args_path}").unwrap();
        writeln!(f, "echo '{{\"code\":0}}'").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }
        script_path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn test_run_cli_inserts_profile_arg() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
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
    #[tokio::test]
    async fn test_send_msg_composite_peer_id_no_reply() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
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
    #[tokio::test]
    async fn test_send_msg_composite_peer_id_thread_reply() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
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
    #[tokio::test]
    async fn test_send_msg_composite_peer_id_message_reply() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
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
    #[tokio::test]
    async fn test_send_msg_simple_peer_id_no_reply() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
        let result = adapter.send_msg("oc_chat123", "text", "hello", None).await;
        assert!(result.is_ok());
        let args_file = tmp.path().join("captured_args");
        let content = std::fs::read_to_string(&args_file).unwrap_or_default();
        assert!(content.contains("--chat-id"));
        assert!(content.contains("oc_chat123"));
        assert!(content.contains("+messages-send"));
    }

    /// Composite peer_id with group chat (oc_) → extract user-id portion.
    #[tokio::test]
    async fn test_send_msg_composite_peer_id_group_chat() {
        let tmp = TempDir::new().unwrap();
        let cli = create_args_echo_cli(&tmp);
        let adapter = make_adapter_with_cli(&cli);
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
}
