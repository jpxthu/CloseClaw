//! Slash Command Parser
//!
//! Parses slash commands like /plan, /code, /review, /debug, /direct, /think
//! and maps them to reasoning modes.

use serde::{Deserialize, Serialize};

pub use crate::session::persistence::ReasoningMode;

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
}
