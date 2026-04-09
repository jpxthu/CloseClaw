//! Slash Command Parser
//!
//! Parses slash commands like /plan, /code, /review, /debug, /direct, /think
//! and maps them to reasoning modes.

use serde::{Deserialize, Serialize};

pub use crate::session::persistence::ReasoningMode;

/// Help text for slash commands
pub const SLASH_HELP_TEXT: &str = r#"可用斜杠指令：
/plan <任务>   - 先规划再执行
/code <任务>   - 生成代码
/review <内容> - 代码审查
/debug <问题>  - 调试分析
/direct        - 直接回答
/think <问题>  - 深度思考
/mode          - 查看当前模式
/mode <模式>   - 切换到指定模式
/compact       - 压缩上下文
/help          - 显示此帮助"#;

/// Slash command handler result
#[derive(Debug, Clone)]
pub enum SlashCommandResult {
    /// Switch to target mode (for mode-switching commands)
    SwitchMode(ReasoningMode),
    /// Return text response to user (for meta commands)
    Text(String),
    /// Compact context result
    Compact { before: usize, after: usize },
    /// Unknown command error
    Unknown(String),
}

impl SlashCommandResult {
    /// Check if this result switches mode
    pub fn is_mode_switch(&self) -> bool {
        matches!(self, SlashCommandResult::SwitchMode(_))
    }

    /// Get the target mode if this is a mode switch
    pub fn target_mode(&self) -> Option<ReasoningMode> {
        match self {
            SlashCommandResult::SwitchMode(mode) => Some(*mode),
            _ => None,
        }
    }

    /// Get the text response if this is a text result
    pub fn text(&self) -> Option<&str> {
        match self {
            SlashCommandResult::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Get the compact result if this is a compact result
    pub fn compact(&self) -> Option<(usize, usize)> {
        match self {
            SlashCommandResult::Compact { before, after } => Some((*before, *after)),
            _ => None,
        }
    }
}

/// Slash command to mode mapping (used by decision module)
pub const SLASH_MODE_MAP: &[(&str, ReasoningMode)] = &[
    ("/plan", ReasoningMode::Plan),
    ("/code", ReasoningMode::Stream),
    ("/review", ReasoningMode::Plan),
    ("/debug", ReasoningMode::Stream),
    ("/direct", ReasoningMode::Direct),
    ("/think", ReasoningMode::Hidden),
];

/// Slash command parsing result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    /// The command (e.g., "/plan")
    pub command: String,
    /// Arguments after the command
    pub args: String,
    /// Raw user input
    pub raw_input: String,
    /// Target reasoning mode
    pub target_mode: ReasoningMode,
    /// Whether this is a meta command (non-mode-switching)
    pub is_meta_command: bool,
}

impl SlashCommand {
    /// Create a new SlashCommand
    pub fn new(
        command: impl Into<String>,
        args: impl Into<String>,
        raw_input: impl Into<String>,
        target_mode: ReasoningMode,
        is_meta_command: bool,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into(),
            raw_input: raw_input.into(),
            target_mode,
            is_meta_command,
        }
    }
}

/// Slash command to mode mapping
#[derive(Clone)]
pub struct SlashModeMap {
    /// Command to mode mapping
    mode_map: std::collections::HashMap<String, ReasoningMode>,
    /// Meta commands (non-mode-switching)
    meta_commands: std::collections::HashSet<String>,
}

impl SlashModeMap {
    /// Create with known commands
    pub fn new() -> Self {
        let mut mode_map = std::collections::HashMap::new();
        mode_map.insert("/plan".to_string(), ReasoningMode::Plan);
        mode_map.insert("/code".to_string(), ReasoningMode::Stream);
        mode_map.insert("/review".to_string(), ReasoningMode::Plan);
        mode_map.insert("/debug".to_string(), ReasoningMode::Stream);
        mode_map.insert("/direct".to_string(), ReasoningMode::Direct);
        mode_map.insert("/think".to_string(), ReasoningMode::Hidden);

        let mut meta_commands = std::collections::HashSet::new();
        meta_commands.insert("/mode".to_string());
        meta_commands.insert("/compact".to_string());
        meta_commands.insert("/help".to_string());

        Self {
            mode_map,
            meta_commands,
        }
    }



    /// Get the mode for a command
    pub fn get(&self, command: &str) -> Option<ReasoningMode> {
        self.mode_map.get(command).copied()
    }

    /// Check if a command is a meta command
    pub fn is_meta(&self, command: &str) -> bool {
        self.meta_commands.contains(command)
    }

    /// Check if input matches any slash command
    pub fn matches(&self, input: &str) -> bool {
        self.mode_map.contains_key(input) || self.meta_commands.contains(input)
    }
}

impl Default for SlashModeMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a slash command from user input
pub fn parse_slash_command(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();

    // Must start with /
    if !trimmed.starts_with('/') {
        return None;
    }

    // Find end of command (space or end of string)
    let after_slash = &trimmed[1..];
    let space_idx = after_slash.find(|c: char| c.is_whitespace());

    let (command_base, rest) = if let Some(idx) = space_idx {
        (&after_slash[..idx], after_slash[idx..].trim_start())
    } else {
        (after_slash, "")
    };

    // Build the full command (lowercase)
    let command = format!("/{}", command_base.to_lowercase());

    // Meta commands
    let meta_commands = ["/mode", "/compact", "/help"];
    let is_meta_command = meta_commands.contains(&command.as_str());

    // Get mode from SLASH_MODE_MAP
    let slash_map = SlashModeMap::new();
    let target_mode = if is_meta_command {
        ReasoningMode::Direct // Meta commands don't switch mode
    } else {
        *slash_map.mode_map.get(&command).unwrap_or(&ReasoningMode::Direct)
    };

    Some(SlashCommand::new(
        command,
        rest.to_string(),
        input.to_string(),
        target_mode,
        is_meta_command,
    ))
}

/// Handle a parsed slash command and return the result
/// 
/// For mode-switching commands (/plan, /code, etc.), returns SwitchMode.
/// For meta commands (/help, /mode, /compact), returns appropriate result.
/// For unrecognized commands, returns Unknown.
pub fn handle_slash_command(cmd: &SlashCommand) -> SlashCommandResult {
    match cmd.command.as_str() {
        "/help" => SlashCommandResult::Text(SLASH_HELP_TEXT.to_string()),
        
        "/mode" => {
            if cmd.args.is_empty() {
                // /mode without args: caller should provide current mode
                // Return a placeholder that caller will fill in
                SlashCommandResult::Text("请提供模式：direct, plan, stream, hidden".to_string())
            } else {
                // /mode with args: switch to specified mode
                let target = match cmd.args.to_lowercase().as_str() {
                    "direct" => ReasoningMode::Direct,
                    "plan" => ReasoningMode::Plan,
                    "stream" => ReasoningMode::Stream,
                    "hidden" => ReasoningMode::Hidden,
                    _ => {
                        return SlashCommandResult::Text(
                            format!("无效模式。可用模式：direct, plan, stream, hidden")
                        )
                    }
                };
                SlashCommandResult::SwitchMode(target)
            }
        }
        
        "/compact" => {
            // /compact: caller should perform actual compaction
            // Return placeholder with mock values - caller will replace
            SlashCommandResult::Compact { before: 0, after: 0 }
        }
        
        "/plan" | "/code" | "/review" | "/debug" | "/direct" | "/think" => {
            SlashCommandResult::SwitchMode(cmd.target_mode)
        }
        
        _ => SlashCommandResult::Unknown(cmd.command.clone()),
    }
}

/// Format mode for display
pub fn format_mode(mode: ReasoningMode) -> &'static str {
    match mode {
        ReasoningMode::Direct => "direct",
        ReasoningMode::Plan => "plan",
        ReasoningMode::Stream => "stream",
        ReasoningMode::Hidden => "hidden",
    }
}

/// Build a friendly unknown command response
pub fn unknown_command_response(command: &str) -> String {
    format!(
        "未知指令: {}。输入 /help 查看可用指令。",
        command
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_slash_plan() {
        let result = parse_slash_command("/plan 设计一个缓存系统");
        assert!(result.is_some());

        let cmd = result.unwrap();
        assert_eq!(cmd.command, "/plan");
        assert_eq!(cmd.args, "设计一个缓存系统");
        assert_eq!(cmd.target_mode, ReasoningMode::Plan);
        assert!(!cmd.is_meta_command);
    }

    #[test]
    fn test_parse_slash_code() {
        let cmd = parse_slash_command("/code 写一个排序函数").unwrap();
        assert_eq!(cmd.command, "/code");
        assert_eq!(cmd.target_mode, ReasoningMode::Stream);
    }

    #[test]
    fn test_parse_slash_direct() {
        let cmd = parse_slash_command("/direct").unwrap();
        assert_eq!(cmd.command, "/direct");
        assert_eq!(cmd.args, "");
        assert_eq!(cmd.target_mode, ReasoningMode::Direct);
    }

    #[test]
    fn test_parse_slash_think() {
        let cmd = parse_slash_command("/think 分析这个方案的风险").unwrap();
        assert_eq!(cmd.command, "/think");
        assert_eq!(cmd.target_mode, ReasoningMode::Hidden);
    }

    #[test]
    fn test_parse_slash_meta() {
        let cmd = parse_slash_command("/mode stream").unwrap();
        assert_eq!(cmd.command, "/mode");
        assert!(cmd.is_meta_command);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let cmd = parse_slash_command("/PLAN 设计系统").unwrap();
        assert_eq!(cmd.command, "/plan");
    }

    #[test]
    fn test_parse_non_slash() {
        let result = parse_slash_command("帮我设计一个系统");
        assert!(result.is_none());
    }

    #[test]
    fn test_slash_mode_map() {
        let map = SlashModeMap::new();
        assert_eq!(map.get("/plan"), Some(ReasoningMode::Plan));
        assert_eq!(map.get("/code"), Some(ReasoningMode::Stream));
        assert_eq!(map.get("/direct"), Some(ReasoningMode::Direct));
        assert!(map.is_meta("/mode"));
        assert!(!map.is_meta("/plan"));
    }

    // === Handler tests ===

    #[test]
    fn test_handle_slash_plan_switches_mode() {
        let cmd = parse_slash_command("/plan 设计系统").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.is_mode_switch());
        assert_eq!(result.target_mode(), Some(ReasoningMode::Plan));
    }

    #[test]
    fn test_handle_slash_code_switches_mode() {
        let cmd = parse_slash_command("/code 写函数").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.is_mode_switch());
        assert_eq!(result.target_mode(), Some(ReasoningMode::Stream));
    }

    #[test]
    fn test_handle_slash_direct_switches_mode() {
        let cmd = parse_slash_command("/direct").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.is_mode_switch());
        assert_eq!(result.target_mode(), Some(ReasoningMode::Direct));
    }

    #[test]
    fn test_handle_slash_think_switches_mode() {
        let cmd = parse_slash_command("/think 分析风险").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.is_mode_switch());
        assert_eq!(result.target_mode(), Some(ReasoningMode::Hidden));
    }

    #[test]
    fn test_handle_help_returns_text() {
        let cmd = parse_slash_command("/help").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.text().is_some());
        let text = result.text().unwrap();
        assert!(text.contains("/plan"));
        assert!(text.contains("/code"));
        assert!(text.contains("/help"));
    }

    #[test]
    fn test_handle_mode_without_args_returns_usage() {
        let cmd = parse_slash_command("/mode").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.text().is_some());
        let text = result.text().unwrap();
        assert!(text.contains("请提供模式"));
    }

    #[test]
    fn test_handle_mode_with_valid_arg_switches_mode() {
        let cmd = parse_slash_command("/mode plan").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.is_mode_switch());
        assert_eq!(result.target_mode(), Some(ReasoningMode::Plan));
    }

    #[test]
    fn test_handle_mode_with_invalid_arg_returns_error() {
        let cmd = parse_slash_command("/mode invalid_mode").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.text().is_some());
        let text = result.text().unwrap();
        assert!(text.contains("无效模式"));
    }

    #[test]
    fn test_handle_compact_returns_compact_result() {
        let cmd = parse_slash_command("/compact").unwrap();
        let result = handle_slash_command(&cmd);
        assert!(result.compact().is_some());
    }

    #[test]
    fn test_handle_unknown_command_returns_unknown() {
        let cmd = parse_slash_command("/unknown").unwrap();
        let result = handle_slash_command(&cmd);
        match result {
            SlashCommandResult::Unknown(cmd_name) => assert_eq!(cmd_name, "/unknown"),
            _ => panic!("Expected Unknown result"),
        }
    }

    #[test]
    fn test_unknown_command_response_format() {
        let response = unknown_command_response("/foo");
        assert!(response.contains("/foo"));
        assert!(response.contains("/help"));
    }

    #[test]
    fn test_format_mode() {
        assert_eq!(format_mode(ReasoningMode::Direct), "direct");
        assert_eq!(format_mode(ReasoningMode::Plan), "plan");
        assert_eq!(format_mode(ReasoningMode::Stream), "stream");
        assert_eq!(format_mode(ReasoningMode::Hidden), "hidden");
    }
}
