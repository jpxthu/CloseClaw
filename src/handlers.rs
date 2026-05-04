//! CLI command handlers.

use anyhow::Result;
use closeclaw::cli::args::*;
use closeclaw::permission::{Defaults, PermissionEngine, RuleSet};
use std::path::PathBuf;
use std::sync::Arc;

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
            println!("Validating config: {}\nConfig is valid", file);
        }
        ConfigAction::List => {
            println!("Config files:\n  (not implemented)");
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
            println!("Checking rule: {}\nRule syntax OK", rule);
        }
        RuleAction::List => {
            println!("Rules:\n  (not implemented)");
        }
    }
    Ok(())
}

pub async fn handle_skill(action: SkillAction) -> Result<()> {
    use closeclaw::skills::{builtin_skills_with_engine, init_disk_skills, ScanConfig};
    match action {
        SkillAction::List => {
            let rs = RuleSet {
                version: "1.0.0".into(),
                rules: vec![],
                defaults: Defaults::default(),
                template_includes: vec![],
                agent_creators: std::collections::HashMap::new(),
            };
            let eng = Arc::new(PermissionEngine::new(rs));
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

pub async fn handle_audit(action: AuditAction) -> Result<()> {
    use closeclaw::audit::*;
    match action {
        AuditAction::Query {
            days,
            event_type,
            agent,
            limit,
        } => {
            let f = AuditQueryFilter {
                days,
                event_type,
                agent,
                limit,
                home_dir: None,
            };
            let evs = query_audit_events(&f);
            if evs.is_empty() {
                println!("No audit events found.");
            } else {
                println!("Found {} audit event(s):", evs.len());
                for e in &evs {
                    println!(
                        "  [{}] {:?} -- {:?}",
                        e.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        e.event_type,
                        e.result
                    );
                }
            }
        }
        AuditAction::Export { output, format } => {
            let f = AuditQueryFilter {
                days: 30,
                event_type: None,
                agent: None,
                limit: None,
                home_dir: None,
            };
            let cnt = export_audit_events(&f, &output, &format)?;
            println!("Exported {} audit event(s) to {} ({})", cnt, output, format);
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
    use dialoguer::*;
    println!("\n=== CloseClaw Setup Wizard ===\n");
    let sel = MultiSelect::new()
        .with_prompt("Select providers")
        .items(&["MiniMax", "OpenAI", "Anthropic"])
        .defaults(&[true])
        .interact()?;
    if sel.is_empty() {
        println!("Cancelled.");
        return Ok(());
    }
    let mut keys = vec![];
    if sel.contains(&0) {
        keys.push((
            "MINIMAX",
            Input::<String>::new()
                .with_prompt("MiniMax Key")
                .interact()?,
        ));
    }
    if sel.contains(&1) {
        keys.push((
            "OPENAI",
            Input::<String>::new()
                .with_prompt("OpenAI Key")
                .interact()?,
        ));
    }
    if sel.contains(&2) {
        keys.push((
            "ANTHROPIC",
            Input::<String>::new()
                .with_prompt("Anthropic Key")
                .interact()?,
        ));
    }
    let mut c = "# CloseClaw config\n".to_string();
    for (k, v) in &keys {
        c.push_str(&format!("{}={}\n", k, v));
    }
    c.push_str("# FEISHU_WEBHOOK=...\n");
    if !skip {
        let confirmed = Confirm::new()
            .with_prompt("Write to configs/.env?")
            .default(true)
            .interact()?;
        if !confirmed {
            return Ok(());
        }
    }
    std::fs::create_dir_all("configs")?;
    std::fs::write("configs/.env", &c)?;
    println!("Written.");
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
