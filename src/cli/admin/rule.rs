//! Rule handler functions for CLI admin.

use super::common::{
    config_dir, effect_to_str, json_error, json_output, RuleCheckOutput, RuleListEntry,
    RuleListOutput,
};
use crate::cli::args::RuleAction;
use anyhow::Result;
use closeclaw_permission::{Rule, RuleSet};
use std::path::{Path, PathBuf};

pub async fn handle_rule(action: RuleAction, json: bool) -> Result<()> {
    handle_rule_with(action, config_dir(), json).await
}

pub async fn handle_rule_with(action: RuleAction, config_dir: PathBuf, json: bool) -> Result<()> {
    match action {
        RuleAction::Check { rule } => handle_rule_check(&rule, json),
        RuleAction::List => handle_rule_list(&config_dir, json),
    }
}

fn handle_rule_check(rule: &str, json: bool) -> Result<()> {
    use closeclaw_permission::rules::validation::validate_rule;
    let is_file_path = rule.starts_with('/')
        || rule.starts_with("./")
        || rule.starts_with("../")
        || rule.ends_with(".json");
    let json_str = if is_file_path {
        let path = Path::new(rule);
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

fn handle_rule_list(config_dir: &Path, json: bool) -> Result<()> {
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
