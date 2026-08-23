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

#[derive(Subcommand)]
pub enum SkillAction {
    /// List installed skills
    List,
    /// Install a skill
    Install {
        /// Skill name
        name: String,
    },
    /// Trigger an immediate skill directory rescan
    Rescan,
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
    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        action: SkillAction,
    }

    /// Normal path: `skill rescan` parses to SkillAction::Rescan.
    #[test]
    fn test_skill_rescan_arg_parsing() {
        let cli = TestCli::try_parse_from(["test", "rescan"]).unwrap();
        assert!(matches!(cli.action, SkillAction::Rescan));
    }

    /// Normal path: `skill list` parses to SkillAction::List.
    #[test]
    fn test_skill_list_arg_parsing() {
        let cli = TestCli::try_parse_from(["test", "list"]).unwrap();
        assert!(matches!(cli.action, SkillAction::List));
    }

    /// Normal path: `skill install my-skill` parses with name.
    #[test]
    fn test_skill_install_arg_parsing() {
        let cli = TestCli::try_parse_from(["test", "install", "my-skill"]).unwrap();
        match cli.action {
            SkillAction::Install { name } => assert_eq!(name, "my-skill"),
            _ => panic!("expected SkillAction::Install"),
        }
    }

    /// Error path: unknown subcommand is rejected.
    #[test]
    fn test_skill_unknown_subcommand_rejected() {
        let result = TestCli::try_parse_from(["test", "unknown"]);
        assert!(result.is_err(), "unknown subcommand should fail parsing");
    }
}
