//! CloseClaw Binary Entry Point

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// Stop the CloseClaw daemon
    Stop {},
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
            // Write PID file so `closeclaw stop` can find us
            let pid_path = pid_file_path();
            if let Some(parent) = pid_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&pid_path, std::process::id().to_string())?;
            println!("PID {} written to {}", std::process::id(), pid_path.display());

            // Start the daemon
            let daemon = closeclaw::daemon::Daemon::start(&config_dir).await?;
            daemon.run().await?;
            println!("CloseClaw daemon stopped.");
        }
        Commands::Stop {} => {
            handle_stop().await?;
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

/// Path to the daemon PID file: ~/.closeclaw/daemon.pid
fn pid_file_path() -> PathBuf {
    let home = std::env::var("HOME").expect("Cannot determine home directory (HOME not set)");
    PathBuf::from(home).join(".closeclaw").join("daemon.pid")
}

async fn handle_stop() -> Result<()> {
    let pid_path = pid_file_path();

    let pid: u32 = if pid_path.exists() {
        let content = std::fs::read_to_string(&pid_path)?;
        content.trim().parse().map_err(|_| anyhow::anyhow!("Invalid PID in {}: {}", pid_path.display(), content))?
    } else {
        anyhow::bail!(
            "PID file not found at {}.\nIs the daemon running? (Hint: use `closeclaw run --config-dir ./configs` to start)",
            pid_path.display()
        );
    };

    // Prevent killing self
    if pid == std::process::id() {
        anyhow::bail!("Refusing to kill self. Use `pkill closeclaw` from another terminal instead.");
    }

    match std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .output()
    {
        Ok(output) if output.status.success() => {
            let _ = std::fs::remove_file(&pid_path);
            println!("✅ Daemon (PID {}) stopped.", pid);
        }
        Ok(output) => {
            anyhow::bail!("kill returned status {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
        }
        Err(e) => anyhow::bail!("Failed to send TERM to PID {}: {}", pid, e),
    }

    Ok(())
}
