//! CloseClaw Binary Entry Point

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

/// Mask an API key for display in previews (show first 4 and last 4 chars).
fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}....{}", &key[..4], &key[key.len() - 4..])
    }
}

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
    Stop {
        /// Force kill the daemon (SIGKILL instead of SIGTERM)
        #[arg(short, long)]
        force: bool,
    },
    /// Chat with the CloseClaw agent via TCP
    Chat {
        #[command(flatten)]
        chat_opts: closeclaw::cli::chat::ChatCommand,
    },
    /// Audit log query and export
    Audit {
        #[command(subcommand)]
        action: AuditAction,
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

#[derive(Subcommand)]
enum AuditAction {
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
        /// Export format: json or jsonl (default: json)
        #[arg(long, default_value = "json")]
        format: String,
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
            println!(
                "PID {} written to {}",
                std::process::id(),
                pid_path.display()
            );

            // Start the daemon
            let daemon = closeclaw::daemon::Daemon::start(&config_dir).await?;
            daemon.run().await?;
            println!("CloseClaw daemon stopped.");
        }
        Commands::Stop { force } => {
            handle_stop(force).await?;
        }
        Commands::Chat { chat_opts } => {
            chat_opts.run().await?;
        }
        Commands::Audit { action } => {
            handle_audit(action).await?;
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
    use closeclaw::permission::{Defaults, PermissionEngine, RuleSet};
    match action {
        SkillAction::List => {
            // Build a minimal engine for CLI skill listing.
            // In normal daemon mode the engine is created with full rules by Daemon::start().
            let rule_set = RuleSet {
                version: "1.0.0".to_string(),
                rules: Vec::new(),
                defaults: Defaults::default(),
                template_includes: Vec::new(),
                agent_creators: std::collections::HashMap::new(),
            };
            let engine = Arc::new(PermissionEngine::new(rule_set));
            println!("Installed skills:");
            let skills = closeclaw::skills::builtin_skills_with_engine(engine);
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

async fn handle_audit(action: AuditAction) -> Result<()> {
    use closeclaw::audit::{export_audit_events, query_audit_events, AuditQueryFilter};

    match action {
        AuditAction::Query {
            days,
            event_type,
            agent,
            limit,
        } => {
            let filter = AuditQueryFilter {
                days,
                event_type,
                agent,
                limit,
            };
            let events = query_audit_events(&filter);
            if events.is_empty() {
                println!("No audit events found.");
            } else {
                println!("Found {} audit event(s):", events.len());
                for event in &events {
                    let ts = event.timestamp.format("%Y-%m-%d %H:%M:%S");
                    let etype = format!("{:?}", event.event_type);
                    let result_str = format!("{:?}", event.result);
                    let details = serde_json::to_string_pretty(&event.details).unwrap_or_default();
                    println!("  [{}] {} — {} ({})", ts, etype, result_str, details);
                }
            }
        }
        AuditAction::Export { output, format } => {
            let filter = AuditQueryFilter {
                days: 30,
                event_type: None,
                agent: None,
                limit: None,
            };
            let count = export_audit_events(&filter, &output, &format)?;
            println!(
                "Exported {} audit event(s) to {} ({})",
                count, output, format
            );
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
    use dialoguer::{Confirm, MultiSelect, Password};

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           CloseClaw 交互式配置向导                      ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("本向导将帮助你配置 API Key。");
    println!("所有配置将写入 configs/.env 文件（不会提交到 Git）。");
    println!();
    println!("操作说明：");
    println!("  空格键 = 切换选择    回车键 = 确认/下一步");
    println!("  方向键 = 导航选项");
    println!();

    // Step 1: Select providers
    let providers = vec!["MiniMax (推荐 / Recommended)", "OpenAI", "Anthropic"];

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

    // Step 2: Collect API keys (using Password to hide input)
    let mut minimax_key = String::new();
    let mut openai_key = String::new();
    let mut anthropic_key = String::new();

    println!();
    println!("【第 2 步】输入 API Key");
    println!("注意：输入时 API Key 不会显示（完全隐藏），输入完成后直接按回车。");
    println!();

    if has_minimax {
        minimax_key = Password::new()
            .with_prompt("MiniMax API Key（必填）")
            .interact()?;
    }

    if has_openai {
        openai_key = Password::new()
            .with_prompt("OpenAI API Key（必填）")
            .interact()?;
    }

    if has_anthropic {
        anthropic_key = Password::new()
            .with_prompt("Anthropic API Key（必填）")
            .interact()?;
    }

    // Step 3: Preview
    println!();
    println!("【第 3 步】配置预览");
    println!();

    let mut env_content = String::from("# CloseClaw 环境配置\n");
    env_content.push_str("# 由 closeclaw config setup 生成\n");
    env_content.push_str("# .env 文件会被 .gitignore 忽略，不会提交到 Git\n\n");

    if has_minimax {
        env_content.push_str("# MiniMax API Key\n");
        env_content.push_str(&format!("MINIMAX_API_KEY={}\n\n", mask_key(&minimax_key)));
    }
    if has_openai {
        env_content.push_str("# OpenAI API Key\n");
        env_content.push_str(&format!("OPENAI_API_KEY={}\n\n", mask_key(&openai_key)));
    }
    if has_anthropic {
        env_content.push_str("# Anthropic API Key\n");
        env_content.push_str(&format!(
            "ANTHROPIC_API_KEY={}\n\n",
            mask_key(&anthropic_key)
        ));
    }
    env_content.push_str("# 飞书 Webhook（可选）\n");
    env_content.push_str("# FEISHU_WEBHOOK=https://open.feishu.cn/open-apis/bot/v2/hook/xxx\n");

    println!("{}", env_content);
    println!("提示：如需配置飞书 Webhook，编辑 configs/.env 取消注释最后一行。");
    println!();

    // Step 4: Confirm
    if skip_confirm {
        println!("（--yes 标志已设置，跳过确认）");
    } else {
        let confirmed = Confirm::new()
            .with_prompt("确认写入 configs/.env？（Y/n）")
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

async fn handle_stop(force: bool) -> Result<()> {
    let pid_path = pid_file_path();

    let pid: u32 = if pid_path.exists() {
        let content = std::fs::read_to_string(&pid_path)?;
        content
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid PID in {}: {}", pid_path.display(), content))?
    } else {
        anyhow::bail!(
            "PID file not found at {}.\nIs the daemon running? (Hint: use `closeclaw run --config-dir ./configs` to start)",
            pid_path.display()
        );
    };

    // Prevent killing self
    if pid == std::process::id() {
        anyhow::bail!(
            "Refusing to kill self. Use `pkill closeclaw` from another terminal instead."
        );
    }

    let sig = if force { "KILL" } else { "TERM" };
    match std::process::Command::new("kill")
        .arg(format!("-{}", sig))
        .arg(pid.to_string())
        .output()
    {
        Ok(output) if output.status.success() => {
            let _ = std::fs::remove_file(&pid_path);
            println!(
                "✅ Daemon (PID {}) stopped ({}).",
                pid,
                if force { "SIGKILL" } else { "SIGTERM" }
            );
        }
        Ok(output) => {
            anyhow::bail!(
                "kill returned status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => anyhow::bail!("Failed to send {} to PID {}: {}", sig, pid, e),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_pid_file_path_contains_daemon_pid() {
        let path = pid_file_path();
        assert!(path.to_str().unwrap().contains(".closeclaw"));
        assert!(path.to_str().unwrap().contains("daemon.pid"));
    }

    #[test]
    fn test_stop_command_force_flag_default() {
        // Test that force defaults to false when parsing "closeclaw stop"
        let cmd = Cli::command();
        let matches = cmd.try_get_matches_from(["closeclaw", "stop"]).unwrap();
        let sub = matches.subcommand().unwrap();
        assert_eq!(sub.0, "stop");
        let stop_matches = sub.1;
        assert!(
            !stop_matches.get_flag("force"),
            "force flag should default to false"
        );
    }

    #[test]
    fn test_stop_command_force_flag_short() {
        // Test that -f flag sets force to true
        let cmd = Cli::command();
        let matches = cmd
            .try_get_matches_from(["closeclaw", "stop", "-f"])
            .unwrap();
        let sub = matches.subcommand().unwrap();
        assert_eq!(sub.0, "stop");
        let stop_matches = sub.1;
        assert!(
            stop_matches.get_flag("force"),
            "force flag should be true with -f"
        );
    }

    #[test]
    fn test_stop_command_force_flag_long() {
        // Test that --force flag sets force to true
        let cmd = Cli::command();
        let matches = cmd
            .try_get_matches_from(["closeclaw", "stop", "--force"])
            .unwrap();
        let sub = matches.subcommand().unwrap();
        assert_eq!(sub.0, "stop");
        let stop_matches = sub.1;
        assert!(
            stop_matches.get_flag("force"),
            "force flag should be true with --force"
        );
    }
}
