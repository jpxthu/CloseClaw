//! Config Wizard - interactive CLI configuration flow

pub mod fetch;
pub mod types;

pub use fetch::*;
pub use types::*;

use crate::agent::config::AgentConfig;
use crate::config::agents::AgentsConfig;
use crate::config::providers::{
    credentials::{AnyProviderCredentials, ApiKeyCredentials},
    models::{ModelDefinition, ModelsConfigData, ProviderConfig},
};
use crate::llm::{
    DeepSeekProvider, GlmProvider, MiniMaxProvider, ModelDiscovery, ModelLister,
    ProviderModelKnowledge, VolcEngineProvider,
};
use dialoguer::{Input, Select};

use std::panic;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use crate::llm::ModelInfo;

/// Parse user input for model selection into a vector of 0-based indices.
///
/// Supported formats:
/// - Space-separated numbers: "1 3 5"
/// - Range syntax: "1-3"
/// - Mixed: "1-3,5,7"
/// - `all` keyword: selects all models
///
/// Returns a sorted, deduplicated vector of indices.
pub fn parse_model_selection(input: &str, total: usize) -> anyhow::Result<Vec<usize>> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("selection cannot be empty");
    }

    if input.eq_ignore_ascii_case("all") {
        return Ok((0..total).collect());
    }

    let mut indices: Vec<usize> = Vec::new();

    // Normalize: replace spaces with commas, then split by comma.
    // This makes both "1 3 5" and "1-3,5,7" work.
    let normalized = input.replace(' ', ",");
    let parts: Vec<&str> = normalized
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for part in parts {
        // Check for range syntax "n-m"
        if part.contains('-') {
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() != 2 {
                anyhow::bail!("invalid range syntax: '{}'", part);
            }
            let start: usize = range_parts[0]
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", range_parts[0].trim()))?;
            let end: usize = range_parts[1]
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", range_parts[1].trim()))?;
            if start == 0 || end == 0 {
                anyhow::bail!("model numbers are 1-based, 0 is not allowed");
            }
            if start > end {
                anyhow::bail!("invalid range: start ({}) > end ({})", start, end);
            }
            if start > total || end > total {
                anyhow::bail!(
                    "model number {} out of range (max is {})",
                    start.max(end),
                    total
                );
            }
            // Expand range "start-end" to all indices in [start, end]
            for idx in start..=end {
                indices.push(idx - 1);
            }
        } else {
            // Single number
            let n: usize = part
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number: '{}'", part))?;
            if n == 0 {
                anyhow::bail!("model numbers are 1-based, 0 is not allowed");
            }
            if n > total {
                anyhow::bail!("model number {} out of range (max is {})", n, total);
            }
            indices.push(n - 1);
        }
    }

    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

/// Locate the project config directory (`~/.closeclaw/`).
fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".closeclaw")
}

/// Merge models from wizard output into existing config, then write models.json.
///
/// Preserves existing providers not being updated and their non-selected models.
fn merge_and_write_models(config_path: &Path, output: &WizardOutput) -> anyhow::Result<PathBuf> {
    let models_path = config_path.join("models.json");
    let existing: ModelsConfigData = if models_path.exists() {
        let content = std::fs::read_to_string(&models_path)?;
        ModelsConfigData::from_json_str(&content).unwrap_or_else(|_| ModelsConfigData::default())
    } else {
        ModelsConfigData::default()
    };

    let new_provider_models: Vec<ModelDefinition> = output
        .selected_models
        .iter()
        .map(|m| ModelDefinition {
            id: m.id.clone(),
            name: Some(m.name.clone()),
            enabled: Some(true),
        })
        .collect();

    let recommended_protocol = output.selected_models.first().map(|m| {
        let kb = ProviderModelKnowledge::new();
        kb.recommended_protocol(&output.provider_id, &m.id)
            .to_string()
    });

    let mut providers = existing.providers;
    providers.remove(&output.provider_id);
    providers.insert(
        output.provider_id.clone(),
        ProviderConfig {
            base_url: None,
            api_key: None,
            api: None,
            protocol: recommended_protocol,
            models: new_provider_models,
        },
    );

    let merged = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    <ModelsConfigData as crate::config::providers::ConfigProvider>::validate(&merged)
        .map_err(|e| anyhow::anyhow!("merged config validation failed: {}", e))?;

    let json = serde_json::to_string_pretty(&merged)?;
    std::fs::write(&models_path, json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", models_path.display(), e))?;
    Ok(models_path)
}

/// Write provider API key to credentials directory.
fn write_provider_credentials(
    config_path: &Path,
    output: &WizardOutput,
) -> anyhow::Result<PathBuf> {
    let creds_dir = config_path.join("credentials");
    std::fs::create_dir_all(&creds_dir)?;
    let cred_file = creds_dir.join(format!("{}.json", output.provider_id));
    let creds = AnyProviderCredentials::ApiKey(ApiKeyCredentials {
        provider: output.provider_id.clone(),
        api_key: output.credential.clone(),
    });
    let creds_json = serde_json::to_string_pretty(&creds)?;
    std::fs::write(&cred_file, creds_json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", cred_file.display(), e))?;
    Ok(cred_file)
}

/// Write wizard output to config files.
///
/// Merging strategy:
/// - New selected models for the chosen provider: add/update with their parameters
/// - Already-configured models for other providers: preserved
/// - Already-configured models for this provider that were NOT selected: preserved
///   (so running wizard multiple times does not lose manual overrides)
///
/// Files written:
/// - `~/.closeclaw/models.json`   — merged provider+model config
/// - `~/.closeclaw/credentials/<provider_id>.json` — API key for the provider
//-
//- NOTE: Any change to `write_wizard_config` or `run_wizard` must be verified
//- against `tests/cli_config_wizard_test.py` (python3 E2E test using pexpect).
pub fn write_wizard_config_to(output: &WizardOutput, config_path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_path)?;

    let models_path = merge_and_write_models(config_path, output)?;
    let cred_file = write_provider_credentials(config_path, output)?;
    let master_config_path = create_master_agent_config(config_path)?;
    let agents_json_path = register_master_in_agents_json(config_path)?;

    println!(
        "✅ Configuration written:\n   models: {}\n   credentials: {}\n   agent: {}\n   agents registry: {}",
        models_path.display(),
        cred_file.display(),
        master_config_path.display(),
        agents_json_path.display()
    );
    Ok(())
}

/// Create master agent directory and config.json.
///
/// If `config.json` already exists, it is not overwritten (idempotent).
fn create_master_agent_config(config_path: &Path) -> anyhow::Result<PathBuf> {
    let agents_dir = config_path.join("agents").join("master");
    let master_config_path = agents_dir.join("config.json");
    if !master_config_path.exists() {
        std::fs::create_dir_all(&agents_dir)
            .map_err(|e| anyhow::anyhow!("failed to create agent dir: {}", e))?;
        let master_config = AgentConfig {
            id: "master".to_string(),
            ..AgentConfig::default()
        };
        master_config
            .save(&master_config_path)
            .map_err(|e| anyhow::anyhow!("failed to write master config: {}", e))?;
    }
    Ok(master_config_path)
}

/// Read or create agents.json, then register `"master"` in it.
///
/// If agents.json already contains `"master"`, no duplicate is added (idempotent).
fn register_master_in_agents_json(config_path: &Path) -> anyhow::Result<PathBuf> {
    let agents_config_dir = config_path.join("config");
    std::fs::create_dir_all(&agents_config_dir)
        .map_err(|e| anyhow::anyhow!("failed to create config dir: {}", e))?;
    let agents_json_path = agents_config_dir.join("agents.json");
    let mut agents_config: AgentsConfig = if agents_json_path.exists() {
        let content = std::fs::read_to_string(&agents_json_path)
            .map_err(|e| anyhow::anyhow!("failed to read agents.json: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse agents.json: {}", e))?
    } else {
        AgentsConfig::default()
    };
    if !agents_config.agents.contains(&"master".to_string()) {
        agents_config.agents.push("master".to_string());
    }
    let agents_json = serde_json::to_string_pretty(&agents_config)
        .map_err(|e| anyhow::anyhow!("failed to serialize agents.json: {}", e))?;
    std::fs::write(&agents_json_path, agents_json)
        .map_err(|e| anyhow::anyhow!("failed to write agents.json: {}", e))?;
    Ok(agents_json_path)
}

/// Write wizard output to config files in the default config directory.
pub fn write_wizard_config(output: &WizardOutput) -> anyhow::Result<()> {
    write_wizard_config_to(output, &config_dir())
}

/// Locate the models.json config file, if it exists.
fn find_models_config() -> Option<std::path::PathBuf> {
    let possible = ["config/models.json", "configs/models.json"];
    possible
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

/// Build an `Arc<dyn ModelLister>` from the selected `ProviderInfo`.
/// Build provider instances and return as a `ModelLister`.
fn build_model_lister(info: &ProviderInfo, credential: &str) -> Arc<dyn ModelLister> {
    match info.provider_type {
        ProviderType::Minimax => Arc::new(MiniMaxProvider::new(credential.to_string())),
        ProviderType::Glm => Arc::new(GlmProvider::new(credential.to_string())),
        ProviderType::Volcengine => Arc::new(VolcEngineProvider::new(credential.to_string())),
        ProviderType::Deepseek => Arc::new(DeepSeekProvider::new(credential.to_string())),
    }
}

/// Run the interactive config wizard.
///
/// Returns `Ok(Some(output))` on success, `Ok(None)` on clean Ctrl+C exit,
/// or an error for invalid input / unexpected failures.
pub async fn run_wizard() -> anyhow::Result<Option<WizardOutput>> {
    // Install panic hook so Ctrl+C (which triggers dialoguer panic) exits cleanly.
    let original_hook = panic::take_hook();
    #[allow(clippy::incompatible_msrv)]
    let hook_box = Box::new(original_hook) as Box<dyn Fn(&panic::PanicHookInfo<'_>) + Send + Sync>;
    panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("canceled") || msg.contains("Aborted") {
            std::process::exit(0);
        }
        hook_box(info);
    }));

    let mut ctx = WizardContext::default();

    // ── SelectProvider ──────────────────────────────────────────────────
    let selection = tokio::task::spawn_blocking(|| {
        Select::new()
            .with_prompt("Select a provider")
            .items(&PROVIDERS.iter().map(|p| p.display_name).collect::<Vec<_>>())
            .default(0)
            .interact()
    })
    .await
    .map_err(|e| anyhow::anyhow!("dialoguer interrupted: {}", e))??;
    ctx.selected_provider = Some(PROVIDERS[selection].clone());

    // ── InputCredential ────────────────────────────────────────────────
    loop {
        let provider_name = ctx
            .selected_provider
            .as_ref()
            .unwrap()
            .display_name
            .to_string();
        let input: String = tokio::task::spawn_blocking(move || {
            dialoguer::Password::new()
                .with_prompt(format!("Enter API token for {}", provider_name))
                .interact()
        })
        .await
        .map_err(|e| anyhow::anyhow!("dialoguer interrupted: {}", e))??;
        if input.trim().is_empty() {
            println!("Token cannot be empty, please try again.");
            continue;
        }
        ctx.credential = Some(input);
        println!("[ OK ] token received");
        break;
    }

    ctx.current_state = WizardState::FetchModels;
    println!("[DEBUG] Transitioned to {:?}", ctx.current_state);

    // ── FetchModels ────────────────────────────────────────────────────
    {
        let info = ctx.selected_provider.as_ref().unwrap();
        let cred = ctx.credential.as_ref().unwrap();
        let model_lister = build_model_lister(info, cred);

        print!("Fetching models from provider...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let discovery = ModelDiscovery::new();
        let ml = Arc::clone(&model_lister);
        let models = discovery
            .discover(info.id, cred, move |cred: &str| {
                let ml = Arc::clone(&ml);
                let cred = cred.to_string();
                async move { ml.fetch_model_list(&cred).await }
            })
            .await;
        println!(" done");
        ctx.fetched_models = models;
    }

    ctx.current_state = WizardState::SelectModels;
    println!("[DEBUG] Transitioned to {:?}", ctx.current_state);

    // ── SelectModels ────────────────────────────────────────────────────
    loop {
        let models = &ctx.fetched_models;
        if models.is_empty() {
            println!("No models available, cannot proceed.");
            return Ok(None);
        }

        // Load existing config if not yet loaded
        if ctx.existing_config.is_empty() {
            if let Some(config_path) = find_models_config() {
                if let Ok(content) = std::fs::read_to_string(&config_path) {
                    if let Ok(json) = serde_json::from_str(&content) {
                        ctx.existing_config = json;
                    }
                }
            }
        }

        // Build display list: [*] for already-configured models
        let display_items: Vec<String> = models
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let is_configured = ctx.existing_config.contains_key(&m.id);
                let mark = if is_configured { "[*]" } else { "[ ]" };
                let reasoning_flag = if m.reasoning { " 🤖" } else { "" };
                let provider_id = ctx.selected_provider.as_ref().unwrap().id;
                let kb = ProviderModelKnowledge::new();
                let proto = kb.recommended_protocol(provider_id, &m.id);
                let proto_str = proto.as_str();
                format!(
                    "{:3}. {} {}{}  protocol: {} (recommended)",
                    i + 1,
                    mark,
                    m.name,
                    reasoning_flag,
                    proto_str
                )
            })
            .collect();

        println!("\n=== Select Models ===");
        println!("Enter model numbers (e.g. 1 3 5, or 1-3,5,7), or 'all' to select everything.");
        println!("Already configured models are marked with [*].\n");
        for item in &display_items {
            println!("{}", item);
        }
        println!();

        let input: String =
            tokio::task::spawn_blocking(|| Input::new().with_prompt("Your selection").interact())
                .await
                .map_err(|e| anyhow::anyhow!("dialoguer interrupted: {}", e))??;

        match parse_model_selection(&input, models.len()) {
            Ok(indices) => {
                ctx.selected_models = indices.iter().map(|&i| models[i].clone()).collect();
                break;
            }
            Err(e) => {
                println!("Invalid selection: {}", e);
                println!("Try again.\n");
                continue;
            }
        }
    }

    ctx.current_state = WizardState::Confirm;
    println!("[DEBUG] Transitioned to {:?}", ctx.current_state);

    // ── Confirm ───────────────────────────────────────────────────────
    loop {
        println!("\n=== Confirm Selection ===");
        println!(
            "{:4} | {:35} | {:>10} | {:>8} | {:>5}",
            "#", "Model", "Context", "MaxTokens", "Temp"
        );
        println!(
            "{:-^4}-+-{:-^37}-+-{:-^10}-+-{:-^8}-+-{:-^5}",
            "", "", "", "", ""
        );

        for (i, model) in ctx.selected_models.iter().enumerate() {
            let temp = model
                .default_temperature
                .map(|t| format!("{:.1}", t))
                .unwrap_or_else(|| "-".to_string());
            let reasoning = if model.reasoning { "✅" } else { "❌" };
            println!(
                "{:4} | {:35} | {:>10} | {:>8} | {:>5} | {}",
                i + 1,
                model.name,
                model.context_window,
                model.max_tokens,
                temp,
                reasoning
            );
        }

        println!();
        let confirm: String = tokio::task::spawn_blocking(|| {
            Input::new()
                .with_prompt("Confirm? (yes/no)")
                .default("yes".into())
                .interact()
        })
        .await
        .map_err(|e| anyhow::anyhow!("dialoguer interrupted: {}", e))??;

        match confirm.trim().to_lowercase().as_str() {
            "yes" | "y" => {
                println!("Confirmed!");
                break;
            }
            "no" | "n" => {
                println!("Going back to model selection...\n");
                ctx.selected_models.clear();
                ctx.current_state = WizardState::SelectModels;
                continue;
            }
            _ => {
                println!("Please enter 'yes' or 'no'.");
                continue;
            }
        }
    }

    // ── WriteConfig ─────────────────────────────────────────────────────
    ctx.current_state = WizardState::WriteConfig;
    println!("[DEBUG] Transitioned to {:?}", ctx.current_state);

    let out_provider_id = ctx.selected_provider.clone().unwrap().id.to_string();
    let out_credential = ctx.credential.clone().unwrap();
    let out_selected_models = ctx.selected_models.clone();

    // Write output to config files
    if let Err(e) = write_wizard_config(&WizardOutput {
        provider_id: out_provider_id.clone(),
        credential: out_credential.clone(),
        selected_models: out_selected_models.clone(),
    }) {
        eprintln!("[ERROR] Failed to write config: {}", e);
        anyhow::bail!("write_wizard_config failed: {}", e);
    }

    Ok(Some(WizardOutput {
        provider_id: out_provider_id,
        credential: out_credential,
        selected_models: out_selected_models,
    }))
}

#[cfg(test)]
mod tests;
