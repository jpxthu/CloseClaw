//! CloseClaw Binary Entry Point

#[cfg(test)]
mod handlers_tests;
use clap::{Parser, Subcommand};
use closeclaw::cli::admin::*;
use closeclaw::cli::args::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "closeclaw", version = env!("CARGO_PKG_VERSION"))]
pub(crate) struct Cli {
    /// Output in JSON format
    #[arg(long, global = true)]
    json: bool,

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
    /// Interactive chat with an agent via the terminal.
    Chat(ChatArgs),
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
        Commands::Agent { action } => handle_agent(action, cli.json).await?,
        Commands::Config { action } => handle_config(action, cli.json).await?,
        Commands::Rule { action } => handle_rule(action, cli.json).await?,
        Commands::Skill { action } => handle_skill(action, cli.json).await?,
        Commands::Chat(args) => closeclaw::cli::chat::run_chat(&args.agent_id).await?,
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
        Commands::Stop { force } => handle_stop(force, cli.json).await?,
    }
    Ok(())
}
