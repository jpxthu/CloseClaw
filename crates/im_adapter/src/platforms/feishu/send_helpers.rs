//! Feishu adapter send helpers — lark-cli subprocess execution.
//!
//! All message sending goes through `lark-cli` subprocess commands:
//! - Text: `lark-cli im +messages-send --chat-id <id> --text "..." --as bot`
//! - Card: `lark-cli im +messages-send --chat-id <id> --msg-type interactive --content '<json>' --as bot`
//! - Reply: `lark-cli im +messages-reply --message-id <id> --text "..." --reply-in-thread`
//! - Media: `lark-cli im +messages-send --chat-id <id> --image <path>` or `--file <path>`
//!
//! HTTP token management and direct API calls are no longer used for sending.

use crate::error::AdapterError;
use super::adapter::FeishuAdapter;

use tokio::process::Command;

/// Execute a lark-cli command and return the result.
///
/// Spawns the subprocess, captures stdout/stderr, and parses the JSON
/// response. Returns `Ok(stdout)` on successful execution, or
/// `Err(SendFailed)` if the process fails to start, exits with error,
/// or returns a non-zero `code` in its JSON output.
pub(crate) async fn run_cli(
    adapter: &FeishuAdapter,
    args: &[&str],
) -> Result<String, AdapterError> {
    let output = Command::new(&adapter.cli_command)
        .args(args)
        .output()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, command = %adapter.cli_command, "failed to spawn lark-cli");
            AdapterError::SendFailed(format!("lark-cli spawn error: {e}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            code = ?output.status.code(),
            stderr = %stderr,
            "lark-cli exited with error"
        );
        return Err(AdapterError::SendFailed(format!(
            "lark-cli exited with code {:?}: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    // Parse JSON response and check for error code.
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(val) => {
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
        Err(_) => {
            // Response is not JSON — likely raw text output. Accept as success.
        }
    }

    Ok(stdout)
}

/// Build lark-cli arguments for sending a message.
///
/// Returns the argument list (without the command name) for:
/// `lark-cli im +messages-send --chat-id <chat_id> --<msg_type_flag> <content> --as bot`
fn build_send_args(chat_id: &str, msg_type: &str, content: &str) -> Vec<String> {
    let mut args = vec![
        "im".to_string(),
        "+messages-send".to_string(),
        "--chat-id".to_string(),
        chat_id.to_string(),
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

/// Build lark-cli arguments for replying to a message in a thread.
///
/// `lark-cli im +messages-reply --message-id <msg_id> --text "..." --reply-in-thread`
fn build_reply_args(message_id: &str, text: &str) -> Vec<String> {
    vec![
        "im".to_string(),
        "+messages-reply".to_string(),
        "--message-id".to_string(),
        message_id.to_string(),
        "--text".to_string(),
        text.to_string(),
        "--reply-in-thread".to_string(),
    ]
}

impl FeishuAdapter {
    /// Low-level: send a message via lark-cli subprocess.
    ///
    /// Routes to `+messages-reply` when `root_id` is `Some` (thread reply),
    /// otherwise to `+messages-send`.
    pub(crate) async fn send_msg(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &str,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let args = match root_id {
            Some(msg_id) => build_reply_args(msg_id, content),
            None => build_send_args(receive_id, msg_type, content),
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
        root_id: Option<&str>,
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
        self.send_msg(chat_id, "text", &plain_text, root_id)
            .await
    }

    /// Send an image file via lark-cli.
    ///
    /// `lark-cli im +messages-send --chat-id <chat_id> --image <path> --as bot`
    #[allow(dead_code)]
    pub(crate) async fn send_image(
        &self,
        chat_id: &str,
        image_path: &str,
    ) -> Result<(), AdapterError> {
        let args = vec![
            "im", "+messages-send", "--chat-id", chat_id,
            "--image", image_path, "--as", "bot",
        ];
        run_cli(self, &args).await?;
        Ok(())
    }

    /// Send a file via lark-cli.
    ///
    /// `lark-cli im +messages-send --chat-id <chat_id> --file <path> --as bot`
    #[allow(dead_code)]
    pub(crate) async fn send_file(
        &self,
        chat_id: &str,
        file_path: &str,
    ) -> Result<(), AdapterError> {
        let args = vec![
            "im", "+messages-send", "--chat-id", chat_id,
            "--file", file_path, "--as", "bot",
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
            "im", "reactions", "create",
            "--params", &params,
            "--data", &data,
        ];
        run_cli(self, &args).await?;
        Ok(())
    }
}

/// Check whether an error is a lark-cli "capability" error.
///
/// These are non-fatal errors where a text fallback is appropriate.
/// For lark-cli, capability errors may come through as specific error
/// codes in the JSON response.
#[allow(dead_code)]
pub(crate) fn is_capability_error(code: i32) -> bool {
    matches!(code, 230001 | 230002)
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
        FeishuAdapter {
            app_id: "test".into(),
            app_secret: "test".into(),
            verification_token: "test".into(),
            http_client: reqwest::Client::new(),
            cached_token: Arc::new(tokio::sync::Mutex::new(None)),
            base_url: "https://open.feishu.cn/open-apis".to_string(),
            last_metadata: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            media_store: Arc::new(
                crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).unwrap(),
            ),
            max_download_size_bytes: u64::MAX,
            workspace_dir: None,
            cli_command: cli_command.to_string(),
        }
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
    fn test_build_reply_args() {
        let args = build_reply_args("om_msg123", "reply text");
        assert_eq!(args[0], "im");
        assert_eq!(args[1], "+messages-reply");
        assert!(args.contains(&"--message-id".to_string()));
        assert!(args.contains(&"om_msg123".to_string()));
        assert!(args.contains(&"--reply-in-thread".to_string()));
    }

    #[test]
    fn test_is_capability_error() {
        assert!(is_capability_error(230001));
        assert!(is_capability_error(230002));
        assert!(!is_capability_error(200));
        assert!(!is_capability_error(0));
        assert!(!is_capability_error(99999));
    }
}
