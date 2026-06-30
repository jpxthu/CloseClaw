//! CloseClaw Binary Entry Point

#[cfg(test)]
mod handlers_tests;
use clap::{Parser, Subcommand};
use closeclaw::cli::admin::*;
use closeclaw::cli::args::*;

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
