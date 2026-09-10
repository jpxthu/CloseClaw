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
    #[arg(short = 'a', long = "agent-id")]
    pub agent_id: String,
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
