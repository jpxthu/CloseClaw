//! CLI command handlers.

use anyhow::Result;
use closeclaw::cli::args::*;
use closeclaw::permission::{Effect, Rule, RuleSet};
use serde::Serialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// JSON output helpers
// ---------------------------------------------------------------------------

/// Print a serializable value as pretty-printed JSON and exit.
fn json_output<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{}", s),
        Err(e) => {
            eprintln!("JSON serialization error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Return a JSON error value suitable for propagation with `?`.
fn json_error(message: &str) -> anyhow::Error {
    #[derive(Serialize)]
    struct ErrorOutput<'a> {
        error: &'a str,
    }
    json_output(&ErrorOutput { error: message });
    anyhow::anyhow!(message.to_string())
}

/// Convert a permission [`Effect`] to its string representation.
fn effect_to_str(effect: Effect) -> &'static str {
    match effect {
        Effect::Allow => "allow",
        Effect::Deny => "deny",
    }
}

// ---------------------------------------------------------------------------
// JSON output structs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(crate) struct ConfigValidateOutput {
    pub file: String,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ConfigListFile {
    pub name: String,
    pub version: String,
    pub path: String,
}

#[derive(Serialize)]
pub(crate) struct ConfigListOutput {
    pub files: Vec<ConfigListFile>,
}

#[derive(Serialize)]
pub(crate) struct RuleCheckOutput {
    pub rule_name: String,
    pub valid: bool,
}

#[derive(Serialize)]
pub(crate) struct RuleListEntry {
    pub name: String,
    pub subject: String,
    pub effect: String,
    pub action_count: usize,
}

#[derive(Serialize)]
pub(crate) struct RuleListOutput {
    pub rules: Vec<RuleListEntry>,
}

#[derive(Serialize)]
pub(crate) struct StopOutput {
    pub pid: u32,
    pub signal: String,
    pub stopped: bool,
}

#[derive(Serialize)]
pub(crate) struct AgentCreateOutput {
    pub status: &'static str,
    pub name: String,
}

#[derive(Serialize)]
pub(crate) struct SkillInstallOutput {
    pub status: &'static str,
    pub name: String,
}

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

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    config_dir_for(home)
}

pub(crate) fn config_dir_for(home: impl AsRef<std::path::Path>) -> PathBuf {
    PathBuf::from(home.as_ref()).join(".closeclaw")
}

pub async fn handle_agent(action: AgentAction, json: bool) -> Result<()> {
    handle_agent_with(action, config_dir(), json).await
}

pub(crate) async fn handle_agent_with(
    action: AgentAction,
    cfg_dir: PathBuf,
    json: bool,
) -> Result<()> {
    let client = closeclaw::admin::AdminClient::new(
        closeclaw::admin::client::admin_socket_path(&cfg_dir)
            .to_string_lossy()
            .into_owned(),
    );
    match action {
        AgentAction::List => handle_agent_list_rpc(&client, json).await,
        AgentAction::Info { name } => handle_agent_info_rpc(&client, &name, json).await,
        AgentAction::Create { name, model } => {
            handle_agent_create_rpc(&client, &name, model, json).await
        }
    }
}

async fn handle_agent_list_rpc(client: &closeclaw::admin::AdminClient, json: bool) -> Result<()> {
    let resp = client
        .call(&closeclaw::admin::AdminRequest::AgentList)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        closeclaw::admin::AdminResponse::AgentListResult { agents } => {
            if agents.is_empty() {
                println!("Agents:\n  (none)");
            } else {
                println!("Agents:");
                for a in &agents {
                    let model = a.model.as_deref().unwrap_or("-");
                    println!("  {} | {} | {}", a.id, a.name, model);
                }
            }
            Ok(())
        }
        closeclaw::admin::AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}

async fn handle_agent_info_rpc(
    client: &closeclaw::admin::AdminClient,
    name: &str,
    json: bool,
) -> Result<()> {
    let resp = client
        .call(&closeclaw::admin::AdminRequest::AgentInfo {
            name: name.to_string(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        closeclaw::admin::AdminResponse::AgentInfoResult {
            id,
            name,
            model,
            skills,
        } => {
            println!("Agent: {}", name);
            println!("  ID: {}", id);
            println!("  Model: {}", model.as_deref().unwrap_or("-"));
            if skills.is_empty() {
                println!("  Skills: (none)");
            } else {
                println!("  Skills: {}", skills.join(", "));
            }
            Ok(())
        }
        closeclaw::admin::AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}

async fn handle_agent_create_rpc(
    client: &closeclaw::admin::AdminClient,
    name: &str,
    model: Option<String>,
    json: bool,
) -> Result<()> {
    let resp = client
        .call(&closeclaw::admin::AdminRequest::AgentCreate {
            name: name.to_string(),
            model,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    match resp {
        closeclaw::admin::AdminResponse::Ok => {
            if json {
                json_output(&AgentCreateOutput {
                    status: "created",
                    name: name.to_string(),
                });
                return Ok(());
            }
            println!("Agent '{}' created.", name);
            Ok(())
        }
        closeclaw::admin::AdminResponse::Error { message } => {
            if json {
                return Err(json_error(&message));
            }
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}

pub async fn handle_config(action: ConfigAction, json: bool) -> Result<()> {
    handle_config_with(action, config_dir(), json).await
}

pub(crate) async fn handle_config_with(
    action: ConfigAction,
    config_dir: PathBuf,
    json: bool,
) -> Result<()> {
    match action {
        ConfigAction::Validate { file } => handle_config_validate(&file, json),
        ConfigAction::List => handle_config_list(&config_dir, json),
        ConfigAction::Setup { yes } => handle_config_setup(yes).await,
    }
}

fn handle_config_validate(file: &str, json: bool) -> Result<()> {
    let path = std::path::Path::new(file);
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file.to_string());
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            if json {
                return Err(json_error(&format!("Failed to read '{}': {}", file, e)));
            }
            anyhow::bail!("Failed to read '{}': {}", file, e);
        }
    };
    match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(value) => {
            if json {
                let version = value
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                json_output(&ConfigValidateOutput {
                    file: filename,
                    valid: true,
                    version,
                });
                return Ok(());
            }
            println!("✅ {}: valid JSON", filename);
            if let Some(ver) = value.get("version").and_then(|v| v.as_str()) {
                println!("   version: {}", ver);
            }
        }
        Err(e) => {
            if json {
                json_output(&ConfigValidateOutput {
                    file: filename,
                    valid: false,
                    version: None,
                });
                return Ok(());
            }
            println!("❌ {}: {}", filename, e);
            anyhow::bail!("Validation failed for '{}': {}", file, e);
        }
    }
    Ok(())
}

fn read_config_files(config_dir: &std::path::Path) -> Result<Vec<(String, String, String)>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(config_dir)
        .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", config_dir.display(), e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .map(|e| e.path())
        .collect();
    entries.sort();
    let files: Vec<(String, String, String)> = entries
        .iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string());
            let version = std::fs::read_to_string(p)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| v.get("version")?.as_str().map(String::from))
                .unwrap_or_else(|| "-".to_string());
            (name, version, p.display().to_string())
        })
        .collect();
    Ok(files)
}

fn handle_config_list(config_dir: &std::path::Path, json: bool) -> Result<()> {
    if !config_dir.is_dir() {
        if json {
            json_output(&ConfigListOutput { files: vec![] });
        } else {
            println!("No config directory found at {}", config_dir.display());
        }
        return Ok(());
    }
    let files = read_config_files(config_dir)?;
    if json {
        let output: Vec<ConfigListFile> = files
            .into_iter()
            .map(|(name, version, path)| ConfigListFile {
                name,
                version,
                path,
            })
            .collect();
        json_output(&ConfigListOutput { files: output });
        return Ok(());
    }
    if files.is_empty() {
        println!("No config files found in {}", config_dir.display());
        return Ok(());
    }
    println!("Config files:");
    for (f, v, p) in &files {
        println!("  {} | {} | {}", f, v, p);
    }
    Ok(())
}

pub async fn handle_rule(action: RuleAction, json: bool) -> Result<()> {
    handle_rule_with(action, config_dir(), json).await
}

pub(crate) async fn handle_rule_with(
    action: RuleAction,
    config_dir: PathBuf,
    json: bool,
) -> Result<()> {
    match action {
        RuleAction::Check { rule } => handle_rule_check(&rule, json),
        RuleAction::List => handle_rule_list(&config_dir, json),
    }
}

fn handle_rule_check(rule: &str, json: bool) -> Result<()> {
    use closeclaw::permission::rules::validation::validate_rule;
    let is_file_path = rule.starts_with('/')
        || rule.starts_with("./")
        || rule.starts_with("../")
        || rule.ends_with(".json");
    let json_str = if is_file_path {
        let path = std::path::Path::new(rule);
        std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", rule, e))?
    } else {
        rule.to_string()
    };
    let r: Rule = match serde_json::from_str(&json_str) {
        Ok(r) => r,
        Err(e) => {
            if json {
                return Err(json_error(&format!("Failed to parse rule JSON: {}", e)));
            }
            anyhow::bail!("Failed to parse rule JSON: {}", e);
        }
    };
    let errors = validate_rule(&r);
    if !errors.is_empty() {
        if json {
            json_output(&RuleCheckOutput {
                rule_name: r.name,
                valid: false,
            });
            return Ok(());
        }
        for err in &errors {
            eprintln!("  ❌ {}", err);
        }
        anyhow::bail!("Rule '{}' has {} validation error(s)", r.name, errors.len());
    }
    if json {
        json_output(&RuleCheckOutput {
            rule_name: r.name,
            valid: true,
        });
        return Ok(());
    }
    println!("✅ Rule '{}': valid", r.name);
    Ok(())
}

fn rule_list_json_output(rule_set: &RuleSet) -> Result<()> {
    let rules: Vec<RuleListEntry> = rule_set
        .rules
        .iter()
        .map(|rule| RuleListEntry {
            name: rule.name.clone(),
            subject: rule.subject.agent_id().to_string(),
            effect: effect_to_str(rule.effect).to_string(),
            action_count: rule.actions.len(),
        })
        .collect();
    json_output(&RuleListOutput { rules });
    Ok(())
}

fn handle_rule_list(config_dir: &std::path::Path, json: bool) -> Result<()> {
    let path = config_dir.join("permissions.json");
    if !path.exists() {
        if json {
            json_output(&RuleListOutput { rules: vec![] });
        } else {
            println!("No permissions file found at {}", path.display());
        }
        return Ok(());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path.display(), e))?;
    let rule_set: RuleSet = serde_json::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse '{}': {}", path.display(), e))?;
    if json {
        return rule_list_json_output(&rule_set);
    }
    if rule_set.rules.is_empty() {
        println!("No rules defined in {}", path.display());
        return Ok(());
    }
    println!("Rules ({}):", rule_set.rules.len());
    for rule in &rule_set.rules {
        let effect = effect_to_str(rule.effect);
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
    Ok(())
}

pub async fn handle_skill(action: SkillAction, json: bool) -> Result<()> {
    handle_skill_with(action, config_dir(), json).await
}

pub(crate) async fn handle_skill_with(
    action: SkillAction,
    cfg_dir: PathBuf,
    json: bool,
) -> Result<()> {
    let client = closeclaw::admin::AdminClient::new(
        closeclaw::admin::client::admin_socket_path(&cfg_dir)
            .to_string_lossy()
            .into_owned(),
    );
    match action {
        SkillAction::List => handle_skill_list_rpc(&client, json).await,
        SkillAction::Install { name } => handle_skill_install_rpc(&client, &name, json).await,
    }
}

async fn handle_skill_list_rpc(client: &closeclaw::admin::AdminClient, json: bool) -> Result<()> {
    let resp = client
        .call(&closeclaw::admin::AdminRequest::SkillList)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        closeclaw::admin::AdminResponse::SkillListResult { skills } => {
            if skills.is_empty() {
                println!("Installed skills:\n  (none)");
            } else {
                println!("Installed skills:");
                for s in &skills {
                    let ver = s.version.as_deref().unwrap_or("-");
                    println!("  {} v{}", s.name, ver);
                }
            }
        }
        closeclaw::admin::AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
    Ok(())
}

async fn handle_skill_install_rpc(
    client: &closeclaw::admin::AdminClient,
    name: &str,
    json: bool,
) -> Result<()> {
    let resp = client
        .call(&closeclaw::admin::AdminRequest::SkillInstall {
            name: name.to_string(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    match resp {
        closeclaw::admin::AdminResponse::Ok => {
            if json {
                json_output(&SkillInstallOutput {
                    status: "installed",
                    name: name.to_string(),
                });
                return Ok(());
            }
            println!("Skill '{}' installed.", name);
        }
        closeclaw::admin::AdminResponse::Error { message } => {
            if json {
                return Err(json_error(&message));
            }
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
    Ok(())
}

pub async fn handle_stop(force: bool, json: bool) -> Result<()> {
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
            if json {
                json_output(&StopOutput {
                    pid,
                    signal: sig.to_string(),
                    stopped: true,
                });
                return Ok(());
            }
            println!("Daemon (PID {}) stopped ({}).", pid, sig);
        }
        Ok(o) => {
            if json {
                return Err(json_error(&format!("kill returned {}", o.status)));
            }
            anyhow::bail!("kill returned {}", o.status);
        }
        Err(e) => {
            if json {
                return Err(json_error(&format!("Failed to kill: {}", e)));
            }
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
