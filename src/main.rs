//! CloseClaw Binary Entry Point

mod handlers;
use crate::handlers::*;
use clap::{Parser, Subcommand};
use closeclaw::cli::args::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "closeclaw", version = env!("CARGO_PKG_VERSION"))]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Rule {
        #[command(subcommand)]
        action: RuleAction,
    },
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    Run {
        #[arg(short, long, default_value = "")]
        config_dir: String,
    },
    Stop {
        #[arg(short, long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    closeclaw::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Agent { action } => handle_agent(action).await?,
        Commands::Config { action } => handle_config(action).await?,
        Commands::Rule { action } => handle_rule(action).await?,
        Commands::Skill { action } => handle_skill(action).await?,
        Commands::Run { config_dir } => {
            let config_dir: PathBuf = if config_dir.is_empty() {
                dirs::home_dir()
                    .map(|h| h.join(".closeclaw"))
                    .unwrap_or_else(|| PathBuf::from(".closeclaw"))
            } else {
                PathBuf::from(config_dir)
            };
            std::fs::create_dir_all(&config_dir).ok();
            let p = pid_file_path();
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).ok();
            }
            std::fs::write(&p, std::process::id().to_string())?;
            println!("PID {} written to {}", std::process::id(), p.display());
            closeclaw::daemon::Daemon::start(config_dir.to_string_lossy().as_ref())
                .await?
                .run()
                .await?;
            println!("Daemon stopped.");
        }
        Commands::Stop { force } => handle_stop(force).await?,
    }
    Ok(())
}
