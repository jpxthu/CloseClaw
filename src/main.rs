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
    /// Interactive setup wizard
    Setup {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
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
        ConfigAction::Setup { yes } => {
            handle_config_setup(yes).await?;
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

async fn handle_config_setup(skip_confirm: bool) -> Result<()> {
    use dialoguer::{MultiSelect, Input, Confirm};

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           CloseClaw 交互式配置向导                      ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("本向导将帮助你配置 API Key 和其他设置。");
    println!("所有配置将写入 configs/.env 文件（不会提交到 Git）。");
    println!();

    // Step 1: Select providers
    let providers = vec![
        "MiniMax (推荐 / Recommended)",
        "OpenAI",
        "Anthropic",
    ];

    println!("【第 1 步】选择要配置的 LLM Provider");
    println!("提示：至少选择一个 Provider。你可以选择多个。");
    println!();

    let selection = MultiSelect::new()
        .with_prompt("选择要配置的 Provider（空格选择，回车确认）")
        .items(&providers)
        .defaults(&[true, false, false]) // MiniMax selected by default
        .interact()?;

    if selection.is_empty() {
        println!("你没有选择任何 Provider，取消配置。");
        return Ok(());
    }

    let has_minimax = selection.contains(&0);
    let has_openai = selection.contains(&1);
    let has_anthropic = selection.contains(&2);

    // Step 2: Collect API keys
    let mut minimax_key = String::new();
    let mut openai_key = String::new();
    let mut anthropic_key = String::new();

    println!();
    println!("【第 2 步】输入 API Key");
    println!("提示：API Key 不会显示在屏幕上，输入时注意不要复制多余的空格。");
    println!();

    if has_minimax {
        minimax_key = Input::new()
            .with_prompt("MiniMax API Key（必填）")
            .interact_text()?;
    }

    if has_openai {
        openai_key = Input::new()
            .with_prompt("OpenAI API Key（必填）")
            .interact_text()?;
    }

    if has_anthropic {
        anthropic_key = Input::new()
            .with_prompt("Anthropic API Key（必填）")
            .interact_text()?;
    }

    // Step 3: Optional Feishu webhook
    println!();
    println!("【第 3 步】飞书 Webhook（可选）");
    println!("如果你需要接收通知，可以配置飞书机器人 Webhook。");
    println!("留空跳过。");

    let feishu_webhook: String = Input::new()
        .with_prompt("飞书 Webhook URL（可选）")
        .allow_empty(true)
        .interact_text()?;

    // Step 4: Preview
    println!();
    println!("【第 4 步】配置预览");
    println!();

    let mut env_content = String::from("# CloseClaw 环境配置\n");
    env_content.push_str("# 由 closeclaw config setup 生成\n");
    env_content.push_str("# .env 文件会被 .gitignore 忽略，不会提交到 Git\n\n");

    if has_minimax {
        env_content.push_str(&format!("# MiniMax API Key（必填）\n"));
        env_content.push_str(&format!("MINIMAX_API_KEY={}\n\n", minimax_key));
    }
    if has_openai {
        env_content.push_str(&format!("# OpenAI API Key（可选）\n"));
        env_content.push_str(&format!("OPENAI_API_KEY={}\n\n", openai_key));
    }
    if has_anthropic {
        env_content.push_str(&format!("# Anthropic API Key（可选）\n"));
        env_content.push_str(&format!("ANTHROPIC_API_KEY={}\n\n", anthropic_key));
    }
    if !feishu_webhook.trim().is_empty() {
        env_content.push_str(&format!("# 飞书 Webhook（可选）\n"));
        env_content.push_str(&format!("FEISHU_WEBHOOK={}\n", feishu_webhook.trim()));
    }

    println!("{}", env_content);

    // Step 5: Confirm
    if skip_confirm {
        println!("（--yes 标志已设置，跳过确认）");
    } else {
        let confirmed = Confirm::new()
            .with_prompt("确认写入 configs/.env？")
            .default(true)
            .interact()?;

        if !confirmed {
            println!("取消配置。");
            return Ok(());
        }
    }

    // Write to file
    let config_dir = PathBuf::from("./configs");
    let env_path = config_dir.join(".env");

    // Create configs dir if needed
    std::fs::create_dir_all(&config_dir)?;

    std::fs::write(&env_path, &env_content)?;

    println!();
    println!("✅ 配置已写入 {}", env_path.display());
    println!();
    println!("下一步：");
    println!("  1. 编辑 configs/agents.json 配置你的 Agent");
    println!("  2. 运行 cargo run --release -- run 启动 Daemon");
    println!();

    Ok(())
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
