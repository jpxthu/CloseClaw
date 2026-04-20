//! CLI argument types for CloseClaw commands.

use clap::Subcommand;

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
        /// Agent name
        name: String,
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
}

#[derive(Subcommand)]
pub enum AuditAction {
    /// Query audit logs
    Query {
        /// Number of past days to search (default: 1)
        #[arg(long, default_value = "1")]
        days: u32,
        /// Filter by event type (e.g. "permission_check", "agent_start")
        #[arg(long)]
        event_type: Option<String>,
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Export audit logs to a file
    Export {
        /// Output file path
        #[arg(long)]
        output: String,
        /// Output format (json or csv)
        #[arg(long, default_value = "json")]
        format: String,
    },
}
