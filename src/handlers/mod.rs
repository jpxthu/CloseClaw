//! CLI command handlers.

use anyhow::Result;
use closeclaw::cli::args::*;
use closeclaw::config::agents::{validate_agents_config, AgentsConfig};
use closeclaw::config::providers::channels::ChannelsConfigData;
use closeclaw::config::providers::gateway::GatewayConfigData;
use closeclaw::config::providers::models::ModelsConfigData;
use closeclaw::config::providers::plugins::PluginsConfigData;
use closeclaw::config::providers::system::SystemConfigData;
use closeclaw::config::{ConfigManager, ConfigProvider};
use closeclaw::permission::rules::validation::validate_rule;
use closeclaw::permission::{Defaults, PermissionEngine, Rule, RuleSet};
use std::path::PathBuf;
use std::sync::Arc;

#[allow(dead_code)] // pub API for masking secrets in CLI output (covered by tests)
pub fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}....{}", &key[..4], &key[key.len() - 4..])
    }
}

pub fn pid_file_path() -> PathBuf {
    pid_file_path_at(&dirs::home_dir().expect("HOME not set"))
}

#[allow(dead_code)]
pub fn pid_file_path_at(home: &std::path::Path) -> PathBuf {
    home.join(".closeclaw").join("daemon.pid")
}

pub async fn handle_agent(action: AgentAction, user_id: &str) -> Result<()> {
    let _owner = user_id; // reserved for permission checks
    match action {
        AgentAction::List => {
            println!("Agents:\n  (no agents running)");
        }
        AgentAction::Create { name, model } => {
            let m = model.unwrap_or_else(|| "minimax/MiniMax-M2.7".to_string());
            println!("Creating agent '{}' with model '{}'", name, m);
        }
        AgentAction::Info { name } => {
            println!("Agent info for '{}':\n  (not implemented)", name);
        }
    }
    Ok(())
}

/// Validate agents config content (JSON value with `agents` key).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not an agents config.
fn validate_agents_config_content(val: &serde_json::Value) -> Result<bool> {
    if val.get("agents").is_none() {
        return Ok(false);
    }
    match serde_json::from_value::<AgentsConfig>(val.clone()) {
        Ok(config) => {
            if let Err(e) = validate_agents_config(&config) {
                anyhow::bail!("agents config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("agents config parse error: {}", e),
    }
}

/// Validate models config content (JSON value with `providers` or `mode` key).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not a models config.
fn validate_models_config_content(val: &serde_json::Value) -> Result<bool> {
    if val.get("providers").is_none() && val.get("mode").is_none() {
        return Ok(false);
    }
    match serde_json::from_value::<ModelsConfigData>(val.clone()) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                anyhow::bail!("models config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("models config parse error: {}", e),
    }
}

/// Validate channels config content (JSON value with `channels` or `bindings` key).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not a channels config.
fn validate_channels_config_content(val: &serde_json::Value) -> Result<bool> {
    if val.get("channels").is_none() && val.get("bindings").is_none() {
        return Ok(false);
    }
    match serde_json::from_value::<ChannelsConfigData>(val.clone()) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                anyhow::bail!("channels config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("channels config parse error: {}", e),
    }
}

/// Validate gateway config content (JSON value with `rateLimitPerMinute` or `maxMessageSize` key).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not a gateway config.
fn validate_gateway_config_content(val: &serde_json::Value) -> Result<bool> {
    if val.get("rateLimitPerMinute").is_none() && val.get("maxMessageSize").is_none() {
        return Ok(false);
    }
    match serde_json::from_value::<GatewayConfigData>(val.clone()) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                anyhow::bail!("gateway config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("gateway config parse error: {}", e),
    }
}

/// Validate plugins config content (JSON value with `entries` and `installs` key).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not a plugins config.
fn validate_plugins_config_content(val: &serde_json::Value) -> Result<bool> {
    if val.get("entries").is_none() || val.get("installs").is_none() {
        return Ok(false);
    }
    match serde_json::from_value::<PluginsConfigData>(val.clone()) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                anyhow::bail!("plugins config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("plugins config parse error: {}", e),
    }
}

/// Validate system config content (JSON value with any of the system-specific keys).
/// Returns `Ok(true)` if recognized, `Ok(false)` if not a system config.
fn validate_system_config_content(val: &serde_json::Value) -> Result<bool> {
    let has_system_key = val.get("wizard").is_some()
        || val.get("update").is_some()
        || val.get("meta").is_some()
        || val.get("messages").is_some()
        || val.get("commands").is_some()
        || val.get("session").is_some()
        || val.get("cron").is_some()
        || val.get("hooks").is_some()
        || val.get("browser").is_some()
        || val.get("auth").is_some();
    if !has_system_key {
        return Ok(false);
    }
    match serde_json::from_value::<SystemConfigData>(val.clone()) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                anyhow::bail!("system config: {}", e);
            }
            Ok(true)
        }
        Err(e) => anyhow::bail!("system config parse error: {}", e),
    }
}

/// Validate config file content by detecting its type and running the appropriate validator.
fn validate_config_content(content: &str) -> Result<()> {
    let val: serde_json::Value =
        serde_json::from_str(content).map_err(|e| anyhow::anyhow!("JSON parse error: {}", e))?;

    match validate_agents_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    match validate_models_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    match validate_channels_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    match validate_gateway_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    match validate_plugins_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    match validate_system_config_content(&val) {
        Ok(true) => {
            println!("Config is valid");
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => anyhow::bail!("{}", e),
    }

    anyhow::bail!("Unrecognized config format: file is not a recognized closeclaw config");
}

pub async fn handle_config(action: ConfigAction) -> Result<()> {
    handle_config_at(action, None).await
}

#[allow(dead_code)]
pub async fn handle_config_at(
    action: ConfigAction,
    home_dir: Option<&std::path::Path>,
) -> Result<()> {
    match action {
        ConfigAction::Validate { file } => {
            let path = std::path::Path::new(&file);
            if !path.exists() {
                anyhow::bail!("Config file not found: {}", file);
            }
            let content = std::fs::read_to_string(path)?;
            validate_config_content(&content)?;
        }
        ConfigAction::List => {
            let config_dir = match home_dir {
                Some(home) => home.join(".closeclaw"),
                None => dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
                    .join(".closeclaw"),
            };
            if !config_dir.exists() {
                println!("Config directory not found at {}", config_dir.display());
                return Ok(());
            }
            let manager = ConfigManager::new(config_dir.clone())?;
            let configs = manager.list_configs();
            if configs.is_empty() {
                println!("No config files found in {}", config_dir.display());
            } else {
                println!("Config files ({}):", configs.len());
                for info in &configs {
                    let version = if info.version.is_empty() {
                        String::from("(no version)")
                    } else {
                        info.version.clone()
                    };
                    let modified = info
                        .last_modified
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "(unknown)".to_string());
                    println!("  {} v{} [{}]", info.path, version, modified);
                }
            }
        }
        ConfigAction::Setup { yes } => {
            handle_config_setup(yes).await?;
        }
    }
    Ok(())
}

pub async fn handle_rule(action: RuleAction, user_id: &str) -> Result<()> {
    let _owner = user_id; // reserved for permission checks
    match action {
        RuleAction::Check { rule } => {
            handle_rule_check(&rule)?;
        }
        RuleAction::List => {
            handle_rule_list()?;
        }
    }
    Ok(())
}

/// Parse a rule string (JSON or YAML) into a Rule struct.
fn parse_rule_input(input: &str) -> Result<Rule> {
    // Try JSON first
    if let Ok(rule) = serde_json::from_str::<Rule>(input) {
        return Ok(rule);
    }
    // Fall back to YAML
    let rule: Rule = serde_yaml::from_str(input)
        .map_err(|e| anyhow::anyhow!("Failed to parse rule as JSON or YAML: {}", e))?;
    Ok(rule)
}

/// Validate a single rule string and print the result.
fn handle_rule_check(rule: &str) -> Result<()> {
    let parsed = parse_rule_input(rule)?;
    let errors = validate_rule(&parsed);
    if errors.is_empty() {
        println!("Rule '{}' is valid", parsed.name);
    } else {
        for err in &errors {
            println!("ERROR: {}", err);
        }
        anyhow::bail!("Rule validation failed for '{}'", parsed.name);
    }
    Ok(())
}

/// List rule files from ~/.closeclaw/rules/ directory.
fn handle_rule_list() -> Result<()> {
    handle_rule_list_at(None)
}

/// List rule files from ~/.closeclaw/rules/ directory, with optional home override.
#[allow(dead_code)]
fn handle_rule_list_at(home_dir: Option<&std::path::Path>) -> Result<()> {
    let rules_dir = match home_dir {
        Some(home) => home.join(".closeclaw").join("rules"),
        None => dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".closeclaw")
            .join("rules"),
    };
    if !rules_dir.exists() {
        println!("No rules directory found at {}", rules_dir.display());
        return Ok(());
    }
    let mut rule_files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&rules_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "json" || ext == "yaml" || ext == "yml" {
                rule_files.push(
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string(),
                );
            }
        }
    }
    rule_files.sort();
    if rule_files.is_empty() {
        println!("No rule files found in {}", rules_dir.display());
    } else {
        println!("Rules ({} files):", rule_files.len());
        for f in &rule_files {
            println!("  {}", f);
        }
    }
    Ok(())
}

pub async fn handle_skill(action: SkillAction, user_id: &str) -> Result<()> {
    let _owner = user_id; // reserved for permission checks
    use closeclaw::skills::{builtin_skills_with_engine, init_disk_skills, ScanConfig};
    match action {
        SkillAction::List => {
            let rs = RuleSet {
                rules: vec![],
                defaults: Defaults::default(),
                template_includes: vec![],
                agent_creators: std::collections::HashMap::new(),
            };
            let eng = Arc::new(PermissionEngine::new_with_default_data_root(rs));
            let config = ScanConfig::default();
            let disk_reg = init_disk_skills(&config);
            if disk_reg.is_empty() {
                println!("Installed skills (bundled):");
            } else {
                println!("Installed skills (disk):");
                for name in disk_reg.list() {
                    println!("  {} [disk]", name);
                }
                println!("Installed skills (bundled):");
            }
            for s in builtin_skills_with_engine(eng).iter() {
                println!(
                    "  {} v{} [bundled]",
                    s.manifest().name,
                    s.manifest().version
                );
            }
        }
        SkillAction::Install { name } => {
            println!("Installing skill: {}", name);
        }
    }
    Ok(())
}

pub async fn handle_stop(force: bool) -> Result<()> {
    handle_stop_at(force, None).await
}

#[allow(dead_code)]
pub async fn handle_stop_at(force: bool, home_dir: Option<&std::path::Path>) -> Result<()> {
    let p = match home_dir {
        Some(home) => pid_file_path_at(home),
        None => pid_file_path(),
    };
    let pid: u32 = if p.exists() {
        std::fs::read_to_string(&p)?
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid PID"))?
    } else {
        anyhow::bail!("PID file not found at {}.", p.display())
    };
    if pid == std::process::id() {
        anyhow::bail!("Refusing to kill self.");
    }
    let sig = if force { "KILL" } else { "TERM" };
    match std::process::Command::new("kill")
        .arg(format!("-{}", sig))
        .arg(pid.to_string())
        .output()
    {
        Ok(o) if o.status.success() => {
            let _ = std::fs::remove_file(&p);
            println!("Daemon (PID {}) stopped ({}).", pid, sig);
        }
        Ok(o) => {
            // kill failed — check if the process is already gone
            let is_alive = std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .output()
                .map(|r| r.status.success())
                .unwrap_or(false);
            if !is_alive {
                // Process already exited; clean up stale PID file
                let _ = std::fs::remove_file(&p);
                println!("Daemon (PID {}) already stopped. PID file cleaned up.", pid);
                return Ok(());
            }
            anyhow::bail!("kill returned {}", o.status);
        }
        Err(e) => {
            anyhow::bail!("Failed to kill: {}", e);
        }
    }
    Ok(())
}

pub async fn handle_config_setup(skip: bool) -> Result<()> {
    use closeclaw::cli::config_wizard;

    println!("\n=== CloseClaw Setup Wizard ===\n");

    let output = match config_wizard::run_wizard().await {
        Ok(Some(output)) => output,
        Ok(None) => {
            println!("Wizard cancelled.");
            return Ok(());
        }
        Err(e) => anyhow::bail!("Wizard error: {}", e),
    };

    // If skip (yes mode), skip the confirm step and write config directly.
    if !skip {
        use dialoguer::Confirm;
        let confirmed = tokio::task::spawn_blocking(|| {
            Confirm::new()
                .with_prompt("Write config now?")
                .default(true)
                .interact()
        })
        .await
        .map_err(|e| anyhow::anyhow!("Confirm task failed: {}", e))??;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    config_wizard::write_wizard_config(&output)?;
    Ok(())
}

#[cfg(test)]
mod handlers_tests;
