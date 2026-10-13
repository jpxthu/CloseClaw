//! Feishu adapter send helpers — lark-cli subprocess execution.
//!
//! All message sending goes through `lark-cli` subprocess commands:
//! - Text: `lark-cli im +messages-send --chat-id <id> --text "..." --as bot`
//! - Card: `lark-cli im +messages-send --chat-id <id> --msg-type interactive
//!   --content '<json>' --as bot`
//! - Reply: `lark-cli im +messages-reply --message-id <id> --text "..." --reply-in-thread`
//! - Media: `lark-cli im +messages-send --chat-id <id> --image <path>` or `--file <path>`
//!
//! HTTP token management and direct API calls are no longer used for sending.

use super::adapter::FeishuAdapter;
use super::outbound_media::prepare_outbound_local_media;
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
/// - P2P (open_id `ou_xxx`):
///   `lark-cli im +messages-send --user-id <id> --<flag> <content> --as bot`
/// - Group (chat_id `oc_xxx`):
///   `lark-cli im +messages-send --chat-id <id> --<flag> <content> --as bot`
pub(crate) fn build_send_args(chat_id: &str, msg_type: &str, content: &str) -> Vec<String> {
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
pub(crate) fn build_reply_args(target: &ReplyTarget, text: &str) -> Vec<String> {
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

// --- Text/reply sending: plain message send + card-to-text fallback ---

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
}

// --- Media sending: image/file upload via lark-cli ---

/// Obtain the process working directory for lark-cli path relativization.
///
/// Returns `None` when `std::env::current_dir()` fails; the failure reason
/// is logged here (warn) and callers pass `None` to `send_media_file`,
/// causing it to fall back to the outbound absolute path.
fn cwd_for_cli() -> Option<std::path::PathBuf> {
    std::env::current_dir()
        .inspect_err(|e| {
            tracing::warn!(error = %e, "Failed to get cwd, passing absolute path to lark-cli");
        })
        .ok()
}

impl FeishuAdapter {
    /// Send an image file via lark-cli.
    ///
    /// - P2P (`ou_xxx`): `lark-cli im +messages-send --user-id <id> --image <path> --as bot`
    /// - Group (`oc_xxx`): `lark-cli im +messages-send --chat-id <id> --image <path> --as bot`
    pub(crate) async fn send_image(
        &self,
        receive_id: &str,
        image_path: &str,
    ) -> Result<(), AdapterError> {
        self.send_media_file(receive_id, "--image", image_path, cwd_for_cli().as_deref())
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
        self.send_media_file(receive_id, "--file", file_path, cwd_for_cli().as_deref())
            .await
    }

    /// Send a media file (image or file) via lark-cli.
    ///
    /// Common implementation for `send_image` and `send_file`.
    /// Validates the file path against the outbound whitelist, copies it
    /// to the outbound directory, and sends the outbound copy.
    /// Returns `Ok(())` and skips sending when:
    /// - `file_path` is an HTTP(S) URL (already platform-hosted)
    /// - File does not exist
    /// - Path fails whitelist validation
    /// - Copy to outbound fails
    ///   (warning logged in all skip cases)
    ///
    /// `cwd` is the working directory used to relativize the outbound path
    /// for lark-cli; `None` falls back to passing the outbound absolute path
    /// (failure reason, if any, is logged by the caller).
    pub(crate) async fn send_media_file(
        &self,
        receive_id: &str,
        media_flag: &str,
        file_path: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<(), AdapterError> {
        let media_store = &self.media_store;
        let workspace_dir = self.workspace_dir.as_deref();
        let outbound_path =
            match prepare_outbound_local_media(file_path, workspace_dir, media_store).await {
                Some(p) => p,
                None => {
                    tracing::warn!(
                        file_path = %file_path,
                        "Skipping media send: not a local file, does not \
                         exist, or failed whitelist validation"
                    );
                    return Ok(());
                }
            };
        // lark-cli only accepts relative paths (design doc: CLI 仅接受进程工作目录的相对路径)
        // Defensive: cwd unavailable (acquisition failure logged by the
        // caller); fall back to the outbound absolute path so
        // `make_media_path_relative` passes it through unchanged.
        let effective_cwd = cwd.unwrap_or(&outbound_path);
        let path_str = make_media_path_relative(&outbound_path, effective_cwd);
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
            &path_str,
            "--as",
            "bot",
        ];
        run_cli(self, &args).await?;
        Ok(())
    }
}

// --- Emoji reaction: reactions create ---

impl FeishuAdapter {
    /// Send an emoji reaction to a message via lark-cli.
    ///
    /// `lark-cli im reactions create --params '{"message_id":"<id>"}'
    /// --data '{"reaction_type":{"emoji_type":"<emoji>"}}'`
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

/// Convert an absolute media path to a relative path when possible.
///
/// lark-cli only accepts relative paths (design doc: CLI 仅接受进程工作目录的相对路径).
/// When `outbound` is a subdirectory of `cwd`, returns a relative path;
/// otherwise returns the original absolute path string.
///
/// # Arguments
/// * `outbound` - Absolute path to the outbound media file.
/// * `cwd` - The working directory to compute the relative path against.
///   Production callers obtain it from `std::env::current_dir()`;
///   tests may inject a synthetic path.
pub(crate) fn make_media_path_relative(
    outbound: &std::path::Path,
    cwd: &std::path::Path,
) -> String {
    match outbound.strip_prefix(cwd) {
        Ok(rel) => {
            let rel_str = rel.to_string_lossy().to_string();
            if rel_str.is_empty() {
                // strip_prefix succeeded but returned empty string,
                // meaning outbound == cwd (not a subdirectory). Preserve
                // absolute path to avoid passing empty string to lark-cli.
                return outbound.to_string_lossy().to_string();
            }
            rel_str
        }
        // outbound is not under cwd — fall back to absolute path
        Err(_) => outbound.to_string_lossy().to_string(),
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
