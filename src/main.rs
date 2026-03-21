//! CloseClaw Binary Entry Point

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "closeclaw")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "CloseClaw - Lightweight, rule-driven multi-agent framework")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all agents
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Validate configuration files
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Check permission rules
    Rule {
        #[command(subcommand)]
        action: RuleAction,
    },
    /// Manage skills
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Run the CloseClaw daemon
    Run {
        /// Configuration directory
        #[arg(short, long, default_value = "./configs")]
        config_dir: String,
    },
}

#[derive(Subcommand)]
enum AgentAction {
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
enum ConfigAction {
    /// Validate a config file
    Validate {
        /// Config file path
        file: String,
    },
    /// List config files
    List,
}

#[derive(Subcommand)]
enum RuleAction {
    /// Check a rule syntax
    Check {
        /// Rule name or file
        rule: String,
    },
    /// List all rules
    List,
}

#[derive(Subcommand)]
enum SkillAction {
    /// List installed skills
    List,
    /// Install a skill
    Install {
        /// Skill name
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    closeclaw::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent { action } => handle_agent(action).await?,
        Commands::Config { action } => handle_config(action).await?,
        Commands::Rule { action } => handle_rule(action).await?,
        Commands::Skill { action } => handle_skill(action).await?,
        Commands::Run { config_dir } => {
            println!("Starting CloseClaw daemon with config dir: {}", config_dir);
            // TODO: Start the daemon
            println!("Daemon not yet implemented");
        }
    }

    Ok(())
}

async fn handle_agent(action: AgentAction) -> Result<()> {
    match action {
        AgentAction::List => {
            println!("Agents:");
            println!("  (no agents running)");
        }
        AgentAction::Create { name, model } => {
            let model = model.unwrap_or_else(|| "minimax/MiniMax-M2.7".to_string());
            println!("Creating agent '{}' with model '{}'", name, model);
            // TODO: Create agent via AgentRegistry
        }
        AgentAction::Info { name } => {
            println!("Agent info for '{}':", name);
            println!("  (not implemented)");
        }
    }
    Ok(())
}

async fn handle_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Validate { file } => {
            println!("Validating config: {}", file);
            // TODO: Load and validate config
            println!("Config is valid");
        }
        ConfigAction::List => {
            println!("Config files:");
            println!("  (not implemented)");
        }
    }
    Ok(())
}

async fn handle_rule(action: RuleAction) -> Result<()> {
    match action {
        RuleAction::Check { rule } => {
            println!("Checking rule: {}", rule);
            // TODO: Check rule syntax
            println!("Rule syntax OK");
        }
        RuleAction::List => {
            println!("Rules:");
            println!("  (not implemented)");
        }
    }
    Ok(())
}

async fn handle_skill(action: SkillAction) -> Result<()> {
    match action {
        SkillAction::List => {
            println!("Installed skills:");
            let skills = closeclaw::skills::builtin_skills();
            for skill in &skills {
                println!("  {} v{}", skill.manifest().name, skill.manifest().version);
            }
        }
        SkillAction::Install { name } => {
            println!("Installing skill: {}", name);
            // TODO: Install skill
        }
    }
    Ok(())
}
