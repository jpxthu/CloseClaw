//! Config handler functions for CLI admin.

use super::common::{
    config_root, json_error, json_output, ConfigListFile, ConfigListOutput, ConfigValidateOutput,
};
use crate::args::ConfigAction;
use anyhow::Result;
use closeclaw_config::reload_manager::filename_to_section;
use closeclaw_config::validators::for_section;
use std::path::{Path, PathBuf};

pub async fn handle_config(action: ConfigAction, json: bool) -> Result<()> {
    handle_config_with(action, config_root()?, json).await
}

pub async fn handle_config_with(
    action: ConfigAction,
    config_dir: PathBuf,
    json: bool,
) -> Result<()> {
    match action {
        ConfigAction::Validate { file } => handle_config_validate(&file, json),
        ConfigAction::List => handle_config_list(&config_dir.join("config"), json),
        ConfigAction::Setup { yes } => handle_config_setup(yes).await,
    }
}

fn handle_config_validate(file: &str, json: bool) -> Result<()> {
    let path = Path::new(file);
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
    // JSON syntax check
    let value = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("JSON parse error: {}", e);
            if json {
                json_output(&ConfigValidateOutput {
                    file: filename,
                    valid: false,
                    version: None,
                    issues: vec![msg],
                    notes: vec![],
                });
                return Ok(());
            }
            println!("❌ {}: {}", filename, msg);
            anyhow::bail!("Validation failed for '{}': {}", file, msg);
        }
    };
    // Schema-level validation for known config sections
    let section = filename_to_section(&filename);
    let mut issues = Vec::new();
    let mut notes = Vec::new();
    match section {
        Some(sec) => {
            let validator = for_section(sec);
            if let Err(msg) = validator(&value) {
                issues.push(msg);
            }
        }
        None => {
            // Unknown config file — syntax-only, informational note
            notes.push(format!(
                "unknown config file '{}'; no schema validation available",
                filename
            ));
        }
    }
    let valid = issues.is_empty();
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from);
    if json {
        json_output(&ConfigValidateOutput {
            file: filename,
            valid,
            version,
            issues,
            notes,
        });
        return Ok(());
    }
    // Text output mode
    if valid {
        println!("✅ {}: valid", filename);
        if let Some(ver) = &version {
            println!("   version: {}", ver);
        }
        for note in &notes {
            println!("   ℹ️  {}", note);
        }
    } else {
        println!("❌ {}: {} issue(s)", filename, issues.len());
        for issue in &issues {
            println!("   - {}", issue);
        }
        for note in &notes {
            println!("   ℹ️  {}", note);
        }
        anyhow::bail!(
            "Validation failed for '{}' with {} issue(s)",
            filename,
            issues.len()
        );
    }
    Ok(())
}

pub fn read_config_files(config_dir: &Path) -> Result<Vec<(String, String, String)>> {
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

fn handle_config_list(config_dir: &Path, json: bool) -> Result<()> {
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

pub async fn handle_config_setup(skip: bool) -> Result<()> {
    use crate::config_wizard;

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

    // Ensure the master agent exists after writing config.
    let config_root = config_root()?;
    let agents_dir = config_root.join("agents");
    let config_dir = config_root.join("config");
    config_wizard::ensure_master_agent(&config_dir, &agents_dir)?;

    Ok(())
}
