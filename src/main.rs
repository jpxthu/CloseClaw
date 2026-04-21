//! CloseClaw Binary Entry Point

mod handlers;
use crate::handlers::*;
use clap::{Parser, Subcommand};
use closeclaw::cli::args::*;
use closeclaw::cli::chat::ChatCommand;

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
        #[arg(short, long, default_value = "./configs")]
        config_dir: String,
    },
    Stop {
        #[arg(short, long)]
        force: bool,
    },
    Chat {
        #[command(flatten)]
        chat_opts: ChatCommand,
    },
    Audit {
        #[command(subcommand)]
        action: AuditAction,
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
            let p = pid_file_path();
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).ok();
            }
            std::fs::write(&p, std::process::id().to_string())?;
            println!("PID {} written to {}", std::process::id(), p.display());
            closeclaw::daemon::Daemon::start(&config_dir)
                .await?
                .run()
                .await?;
            println!("Daemon stopped.");
        }
        Commands::Stop { force } => handle_stop(force).await?,
        Commands::Chat { chat_opts } => chat_opts.run().await?,
        Commands::Audit { action } => handle_audit(action).await?,
    }
    Ok(())
}
