//! CLI argument types for CloseClaw commands.

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum AgentAction {
    /// List all agents
    List,
    /// Create a new agent
    Create {
        /// Agent name
        name: String,
        /// Model to use
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Get agent info
    Info {
        /// Agent ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Validate a config file
    Validate {
        /// Config file path
        file: String,
    },
    /// List config files
    List,
    /// Interactive setup wizard
    Setup {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum RuleAction {
    /// Check a rule syntax
    Check {
        /// Rule name or file
        rule: String,
    },
    /// List all rules
    List,
}

#[derive(Subcommand, Debug)]
pub enum SkillAction {
    /// List installed skills
    List,
}

/// Interactive chat with an agent via the terminal.
#[derive(Args)]
pub struct ChatArgs {
    /// Agent ID to chat with.
    #[arg(short = 'a', long = "agent-id", value_parser = parse_agent_id)]
    pub agent_id: String,
}

/// Validates the `--agent-id` value: an agent ID must not be empty or
/// whitespace-only, otherwise it degrades the session routing key.
fn parse_agent_id(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        Err("--agent-id must not be empty".to_string())
    } else {
        Ok(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wrapper to test subcommand parsing in isolation.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        action: SkillAction,
    }

    /// Normal path: `skill list` parses to SkillAction::List.
    #[test]
    fn test_skill_list_arg_parsing() {
        let cli = TestCli::try_parse_from(["test", "list"]).unwrap();
        assert!(matches!(cli.action, SkillAction::List));
    }

    /// Error path: unknown subcommand is rejected.
    #[test]
    fn test_skill_unknown_subcommand_rejected() {
        let result = TestCli::try_parse_from(["test", "unknown"]);
        assert!(result.is_err(), "unknown subcommand should fail parsing");
    }

    // ── issue #3945 — chat --agent-id 非空校验 ─────────────────────────────

    /// Wrapper to test `ChatArgs` parsing in isolation.
    #[derive(Parser)]
    struct ChatArgsTestCli {
        #[command(flatten)]
        args: ChatArgs,
    }

    /// Error path: `chat -a ""` (empty short value) is rejected at parse time.
    #[test]
    fn test_chat_agent_id_empty_short_value_rejected() {
        let Err(err) = ChatArgsTestCli::try_parse_from(["test", "-a", ""]) else {
            panic!("empty --agent-id should fail parsing");
        };
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("--agent-id"),
            "error should mention --agent-id: {}",
            err_msg
        );
    }

    /// Error path: `chat --agent-id ""` (empty long value) is rejected at parse time.
    #[test]
    fn test_chat_agent_id_empty_long_value_rejected() {
        let Err(err) = ChatArgsTestCli::try_parse_from(["test", "--agent-id", ""]) else {
            panic!("empty --agent-id should fail parsing");
        };
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("--agent-id"),
            "error should mention --agent-id: {}",
            err_msg
        );
    }

    /// Error path: `chat -a "   "` (whitespace-only value) is rejected at parse time.
    #[test]
    fn test_chat_agent_id_whitespace_only_value_rejected() {
        let result = ChatArgsTestCli::try_parse_from(["test", "-a", "   "]);
        assert!(
            result.is_err(),
            "whitespace-only --agent-id should fail parsing"
        );
    }

    /// Normal path: a valid agent id parses and is preserved verbatim, including
    /// leading/trailing whitespace (no trimming; only empty/whitespace-only is rejected).
    #[test]
    fn test_chat_agent_id_valid_value_preserved_as_is() {
        let cli = ChatArgsTestCli::try_parse_from(["test", "-a", "agent-1"]).unwrap();
        assert_eq!(cli.args.agent_id, "agent-1");

        let cli = ChatArgsTestCli::try_parse_from(["test", "-a", " agent-1 "]).unwrap();
        assert_eq!(cli.args.agent_id, " agent-1 ");
    }

    // ── Step 1.3 — skill rescan removal ──────────────────────────────────

    /// skill rescan command must not exist: clap rejects the "rescan" variant
    /// that was removed as part of the design doc alignment (no runtime hot-reload).
    #[test]
    fn test_skill_rescan_arg_parsing_fails() {
        let result = TestCli::try_parse_from(["test", "rescan"]);
        assert!(
            result.is_err(),
            "skill rescan should be rejected by clap (command removed)"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("rescan") || err_msg.contains("unexpected"),
            "error should mention rescan or unexpected subcommand: {}",
            err_msg
        );
    }
}
