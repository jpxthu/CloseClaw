//! CloseClaw Binary Entry Point

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use closeclaw::cli::admin::*;
use closeclaw::cli::args::*;
use closeclaw_permission::{sandbox::run_engine_subprocess, RuleSet};

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

        /// Internal flag: run daemon in the foreground (no child spawn).
        #[arg(long, hide = true, default_value_t = false)]
        foreground: bool,
    },
    Stop {
        #[arg(short, long)]
        force: bool,
    },
}

/// Try to run as sandbox engine subprocess.
///
/// Returns `Some(Ok(()))` if the process entered engine mode and completed,
/// or `Some(Err(..))` if engine mode was entered but failed.
/// Returns `None` if `SANDBOX_ENGINE` is not set, meaning normal CLI flow.
async fn try_run_engine_subprocess() -> Option<anyhow::Result<()>> {
    if std::env::var("SANDBOX_ENGINE").ok().as_deref() != Some("1") {
        return None;
    }
    let ipc_path = std::env::var("SANDBOX_IPC_PATH")
        .expect("SANDBOX_IPC_PATH must be set when SANDBOX_ENGINE=1");
    let rules = RuleSet::default();
    Some(run_engine_subprocess(PathBuf::from(ipc_path), rules).await)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    closeclaw::init();
    if let Some(result) = try_run_engine_subprocess().await {
        return result;
    }
    let cli = Cli::parse();
    match cli.command {
        Commands::Agent { action } => handle_agent(action, cli.json).await?,
        Commands::Config { action } => handle_config(action, cli.json).await?,
        Commands::Rule { action } => handle_rule(action, cli.json).await?,
        Commands::Skill { action } => handle_skill(action, cli.json).await?,
        Commands::Chat(args) => closeclaw::cli::chat::run_chat(&args.agent_id).await?,
        Commands::Run {
            config_dir,
            foreground,
        } => {
            let runner = closeclaw::daemon::bridge::DaemonRunnerImpl;
            handle_run(config_dir, cli.json, foreground, &runner).await?
        }
        Commands::Stop { force } => handle_stop(force, cli.json).await?,
    }
    Ok(())
}
