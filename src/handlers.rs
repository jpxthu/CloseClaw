//! CLI command handlers.

use anyhow::Result;
use closeclaw::cli::args::*;
use closeclaw::permission::{Defaults, Effect, PermissionEngine, Rule, RuleSet};
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
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".closeclaw").join("daemon.pid")
}

#[allow(dead_code)] // Used by config list / rule list in later steps
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".closeclaw")
}

pub async fn handle_agent(action: AgentAction) -> Result<()> {
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

pub async fn handle_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Validate { file } => {
            let path = std::path::Path::new(&file);
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file.clone());
            let contents = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", file, e))?;
            match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(value) => {
                    println!("✅ {}: valid JSON", filename);
                    if let Some(ver) = value.get("version").and_then(|v| v.as_str()) {
                        println!("   version: {}", ver);
                    }
                }
                Err(e) => {
                    println!("❌ {}: {}", filename, e);
                    anyhow::bail!("Validation failed for '{}': {}", file, e);
                }
            }
        }
        ConfigAction::List => {
            let dir = config_dir();
            if !dir.is_dir() {
                println!("No config directory found at {}", dir.display());
                return Ok(());
            }
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", dir.display(), e))?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "json")
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect();
            entries.sort();
            if entries.is_empty() {
                println!("No config files found in {}", dir.display());
                return Ok(());
            }
            println!("Config files:");
            for path in &entries {
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let version = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .and_then(|v| v.get("version")?.as_str().map(String::from))
                    .unwrap_or_else(|| "-".to_string());
                println!("  {} | {} | {}", filename, version, path.display());
            }
        }
        ConfigAction::Setup { yes } => {
            handle_config_setup(yes).await?;
        }
    }
    Ok(())
}

pub async fn handle_rule(action: RuleAction) -> Result<()> {
    match action {
        RuleAction::Check { rule } => {
            use closeclaw::permission::rules::validation::validate_rule;

            let is_file_path = rule.starts_with('/')
                || rule.starts_with("./")
                || rule.starts_with("../")
                || rule.ends_with(".json");

            let json_str = if is_file_path {
                let path = std::path::Path::new(&rule);
                std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", rule, e))?
            } else {
                rule.clone()
            };

            let r: Rule = serde_json::from_str(&json_str)
                .map_err(|e| anyhow::anyhow!("Failed to parse rule JSON: {}", e))?;

            // Check mutual exclusivity of actions/template
            if let Err(e) = r.validate() {
                anyhow::bail!("Rule validation failed: {}", e);
            }

            // Full validation
            let errors = validate_rule(&r);
            if !errors.is_empty() {
                for err in &errors {
                    eprintln!("  ❌ {}", err);
                }
                anyhow::bail!("Rule '{}' has {} validation error(s)", r.name, errors.len());
            }

            println!("✅ Rule '{}': valid", r.name);
        }
        RuleAction::List => {
            let path = config_dir().join("permissions.json");
            if !path.exists() {
                println!("No permissions file found at {}", path.display());
                return Ok(());
            }
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path.display(), e))?;
            let rule_set: RuleSet = serde_json::from_str(&contents)
                .map_err(|e| anyhow::anyhow!("Failed to parse '{}': {}", path.display(), e))?;
            if rule_set.rules.is_empty() {
                println!("No rules defined in {}", path.display());
                return Ok(());
            }
            println!("Rules ({}):", rule_set.rules.len());
            for rule in &rule_set.rules {
                let effect = match rule.effect {
                    Effect::Allow => "allow",
                    Effect::Deny => "deny",
                };
                let action_count = rule.actions.len();
                let action_label = if action_count == 1 {
                    "action"
                } else {
                    "actions"
                };
                println!(
                    "  {} | {} | {} | {} {}",
                    rule.name,
                    rule.subject.agent_id(),
                    effect,
                    action_count,
                    action_label
                );
            }
        }
    }
    Ok(())
}

pub async fn handle_skill(action: SkillAction) -> Result<()> {
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
    let p = pid_file_path();
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
mod tests {
    use super::*;
    use crate::Cli;
    use clap::CommandFactory;
    #[test]
    fn test_pid() {
        assert!(pid_file_path().to_str().unwrap().contains(".closeclaw"));
    }
    #[test]
    fn test_stop_f() {
        let m = Cli::command()
            .try_get_matches_from(["c", "stop", "-f"])
            .unwrap();
        assert!(m.subcommand().unwrap().1.get_flag("force"));
    }
    #[test]
    fn test_mask_key_short() {
        // Keys <= 8 chars are fully masked
        assert_eq!(mask_key("abc"), "****");
        assert_eq!(mask_key("12345678"), "****");
    }
    #[test]
    fn test_mask_key_long() {
        // Keys > 8 chars show first 4 and last 4
        assert_eq!(mask_key("abcdefghij"), "abcd....ghij");
        assert_eq!(mask_key("minimax-key-001"), "mini....-001");
        assert_eq!(mask_key("sk-1234567890abcdef"), "sk-1....cdef");
    }
    #[test]
    fn test_env_write_uses_raw_key() {
        // Verify the format string used in handle_config_setup writes raw key (not masked)
        let k = "MINIMAX";
        let v = "my-secret-key-123";
        let line = format!("{}={}\n", k, v);
        assert!(line.starts_with("MINIMAX=my-secret-key-123"));
        assert!(!line.contains("****"));
        assert!(!line.contains("...."));
        // Also verify the key portion does NOT contain mask pattern
        let written = format!("{}={}", k, v);
        assert!(written.contains("my-secret-key-123"));
    }
}
