//! Slash command handlers for /system, /cd, /pwd, /git
//!
//! These are separate from the mode slash commands (in src/mode/slash_command.rs)
//! and focus on system prompt manipulation.

use super::sections::{clear_append_section, get_append_section, set_append_section as store_append, APPEND_SECTION_MAX_LEN};
use super::workdir::{build_git_status, clear_workdir, get_workdir, set_workdir};
use crate::mode::slash_command::SlashCommandResult;

// ---------------------------------------------------------------------------
// /system command
// ---------------------------------------------------------------------------

/// Handle the `/system <text>` command.
/// Sets the append_section to `<text>`, truncating if over 500 chars.
pub fn handle_system_command(args: &str) -> SlashCommandResult {
    let text = args.trim();

    if text.is_empty() {
        // Show current append section if any
        if let Some(current) = get_append_section() {
            return SlashCommandResult::Text(format!(
                "当前追加内容：\n{}\n\n使用 `/system <内容>` 可更新。",
                current
            ));
        } else {
            return SlashCommandResult::Text(
                "当前无追加内容。使用 `/system <内容>` 添加。".to_string(),
            );
        }
    }

    let truncation_warning = store_append(text.to_string());

    let response = if truncation_warning.is_some() {
        format!(
            "已设置追加内容（已截断至 {} 字）：\n{}\n\n请求结束后自动清除。",
            APPEND_SECTION_MAX_LEN,
            get_append_section().unwrap_or_default()
        )
    } else {
        format!(
            "已设置追加内容：\n{}\n\n请求结束后自动清除。",
            text
        )
    };

    SlashCommandResult::Text(response)
}

// ---------------------------------------------------------------------------
// /cd command
// ---------------------------------------------------------------------------

/// Handle the `/cd <path>` command.
/// Switches the current working directory.
pub fn handle_cd_command(args: &str) -> SlashCommandResult {
    let path = args.trim();

    if path.is_empty() {
        return SlashCommandResult::Text("用法：/cd <路径>".to_string());
    }

    let ctx = set_workdir(path.to_string());

    let git_info = if ctx.has_git {
        format!(
            "\n  branch: {}\n  uncommitted: {}",
            ctx.branch.as_deref().unwrap_or("?"),
            ctx.recent_changes
        )
    } else {
        String::new()
    };

    SlashCommandResult::Text(format!(
        "工作目录已切换：{}\n  git repo: {}{}",
        ctx.path,
        if ctx.has_git { "是" } else { "否" },
        git_info
    ))
}

// ---------------------------------------------------------------------------
// /pwd command
// ---------------------------------------------------------------------------

/// Handle the `/pwd` command.
/// Returns the current working directory.
pub fn handle_pwd_command() -> SlashCommandResult {
    match get_workdir() {
        Some(path) => SlashCommandResult::Text(format!("当前工作目录：{}", path)),
        None => SlashCommandResult::Text("未设置工作目录。使用 /cd <路径> 切换。".to_string()),
    }
}

// ---------------------------------------------------------------------------
// /git command
// ---------------------------------------------------------------------------

/// Handle the `/git <args>` command.
/// Delegates to the actual git binary.
pub fn handle_git_command(args: &str) -> SlashCommandResult {
    let workdir = match get_workdir() {
        Some(w) => w,
        None => {
            return SlashCommandResult::Text(
                "未设置工作目录。使用 /cd <路径> 先切换。".to_string()
            );
        }
    };

    if !std::path::Path::new(&workdir).join(".git").exists() {
        return SlashCommandResult::Text("当前目录不是 git 仓库。".to_string());
    }

    let git_args: Vec<&str> = args
        .trim()
        .split_whitespace()
        .collect();

    if git_args.is_empty() || git_args[0] == "status" {
        // Return embedded git status
        if let Some(status) = build_git_status() {
            SlashCommandResult::Text(status)
        } else {
            SlashCommandResult::Text("无法读取 git 状态。".to_string())
        }
    } else {
        // Delegate to git binary
        use std::process::Command;

        let output = Command::new("git")
            .args(&git_args)
            .current_dir(&workdir)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.success() {
                    SlashCommandResult::Text(stdout.to_string())
                } else {
                    SlashCommandResult::Text(format!(
                        "git {} 失败：\n{}\n{}",
                        git_args.join(" "),
                        stdout,
                        stderr
                    ))
                }
            }
            Err(e) => SlashCommandResult::Text(format!("无法执行 git：{}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_command_set_content() {
        clear_append_section();
        let result = handle_system_command("test append");
        match result {
            SlashCommandResult::Text(t) => {
                assert!(t.contains("已设置追加内容"));
            }
            _ => panic!("Expected Text result"),
        }
        clear_append_section();
    }

    #[test]
    fn test_system_command_empty_shows_current() {
        clear_append_section();
        let result = handle_system_command("");
        match result {
            SlashCommandResult::Text(t) => assert!(t.contains("无追加内容")),
            _ => panic!("Expected Text result"),
        }
        clear_append_section();
    }

    #[test]
    fn test_system_command_truncation() {
        clear_append_section();
        let long_text = "x".repeat(600);
        let result = handle_system_command(&long_text);
        match result {
            SlashCommandResult::Text(t) => {
                assert!(t.contains("已截断") || t.contains("500"));
            }
            _ => panic!("Expected Text result"),
        }
        clear_append_section();
    }

    #[test]
    fn test_pwd_command_no_workdir() {
        clear_workdir();
        let result = handle_pwd_command();
        match result {
            SlashCommandResult::Text(t) => assert!(t.contains("未设置工作目录")),
            _ => panic!("Expected Text result"),
        }
        clear_workdir();
    }

    #[test]
    fn test_cd_command_empty_args() {
        let result = handle_cd_command("");
        match result {
            SlashCommandResult::Text(t) => assert!(t.contains("用法")),
            _ => panic!("Expected Text result"),
        }
    }

    #[test]
    fn test_git_command_no_workdir() {
        clear_workdir();
        let result = handle_git_command("status");
        match result {
            SlashCommandResult::Text(t) => assert!(t.contains("未设置工作目录")),
            _ => panic!("Expected Text result"),
        }
        clear_workdir();
    }
}
