//! Config Wizard - interactive CLI configuration flow

pub mod fetch;
pub mod types;

pub use fetch::*;
pub use types::*;

use crate::agent::config::AgentConfig;
use crate::config::agents::{AgentsConfig, AgentsConfigProvider};
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

/// Locate the project config directory (`~/.closeclaw/config/`).
fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".closeclaw").join("config")
}

#[allow(dead_code)]
/// Create the initial `master` agent if it does not already exist.
///
/// Writes:
/// - `~/.closeclaw/agents/master/config.json`
/// - `~/.closeclaw/config/agents.json` (appends "master" if missing)
///
/// Idempotent: skips files that already exist.
fn ensure_master_agent(config_dir: &Path, agents_dir: &Path) -> anyhow::Result<()> {
    // ── Write master agent config.json if missing ──────────────────────────────
    let master_config_path = agents_dir.join("master").join("config.json");
    if !master_config_path.exists() {
        let dir = master_config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("cannot determine agent directory"))?;
        std::fs::create_dir_all(dir)?;

        let config = AgentConfig {
            id: "master".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| anyhow::anyhow!("failed to serialize AgentConfig: {}", e))?;
        std::fs::write(&master_config_path, json)?;
    }

    // ── Update agents.json registration list ───────────────────────────────────
    let agents_json_path = config_dir.join("agents.json");
    if agents_json_path.exists() {
        let content = std::fs::read_to_string(&agents_json_path)?;
        let provider = AgentsConfigProvider::from_json_str(&content)?;
        let mut agents_config = provider.inner().clone();
        if !agents_config.agents.contains(&"master".to_string()) {
            agents_config.agents.push("master".to_string());
            let json = serde_json::to_string_pretty(&agents_config)
                .map_err(|e| anyhow::anyhow!("failed to serialize agents.json: {}", e))?;
            std::fs::write(&agents_json_path, json)?;
        }
    } else {
        let agents_config = AgentsConfig {
            agents: vec!["master".to_string()],
        };
        let json = serde_json::to_string_pretty(&agents_config)
            .map_err(|e| anyhow::anyhow!("failed to serialize agents.json: {}", e))?;
        std::fs::write(&agents_json_path, json)?;
    }

    Ok(())
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
/// - `~/.closeclaw/config/models.json`   — merged provider+model config
/// - `~/.closeclaw/config/credentials/<provider_id>.json` — API key for the provider
//-
//- NOTE: Any change to `write_wizard_config` or `run_wizard` must be verified
//- against `tests/cli_config_wizard_test.py` (python3 E2E test using pexpect).
pub fn write_wizard_config_to(output: &WizardOutput, config_path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_path)?;

    // ── Load or create models.json ─────────────────────────────────────────────
    let models_path = config_path.join("models.json");
    let existing: ModelsConfigData = if models_path.exists() {
        let content = std::fs::read_to_string(&models_path)?;
        ModelsConfigData::from_json_str(&content).unwrap_or_else(|_| ModelsConfigData::default())
    } else {
        ModelsConfigData::default()
    };

    // ── Build the new provider config from WizardOutput.selected_models ────────
    let new_provider_models: Vec<ModelDefinition> = output
        .selected_models
        .iter()
        .map(|m| ModelDefinition {
            id: m.id.clone(),
            name: Some(m.name.clone()),
            enabled: Some(true),
        })
        .collect();

    // ── Query recommended_protocol from knowledge base ────────────────────────
    let recommended_protocol = output.selected_models.first().map(|m| {
        let kb = ProviderModelKnowledge::new();
        kb.recommended_protocol(&output.provider_id, &m.id)
            .to_string()
    });

    // ── Merge: preserve all existing providers, replace selected provider ───────
    let mut providers = existing.providers;
    // Save existing config before removal so we can inherit fields
    let existing_provider = providers.get(&output.provider_id).cloned();
    // Remove entries for the selected provider so we can replace them
    providers.remove(&output.provider_id);

    // Inherit base_url, api_key, api from existing config if present
    let (base_url, api_key, api) = existing_provider
        .map(|ep| (ep.base_url, ep.api_key, ep.api))
        .unwrap_or((None, None, None));

    let new_provider_config = ProviderConfig {
        base_url,
        api_key,
        api,
        protocol: recommended_protocol,
        credential_path: Some(format!("credentials/{}.json", output.provider_id)),
        models: new_provider_models,
    };
    providers.insert(output.provider_id.clone(), new_provider_config);

    let merged = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    <ModelsConfigData as crate::config::providers::ConfigProvider>::validate(&merged)
        .map_err(|e| anyhow::anyhow!("merged config validation failed: {}", e))?;

    // ── Write models.json ─────────────────────────────────────────────────────
    let json = serde_json::to_string_pretty(&merged)?;
    std::fs::write(&models_path, json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", models_path.display(), e))?;

    // ── Write credentials ────────────────────────────────────────────────────
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

    println!(
        "✅ Configuration written:\n   models: {}\n   credentials: {}",
        models_path.display(),
        cred_file.display()
    );
    Ok(())
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
        let result = discovery
            .discover(info.id, cred, move |cred: &str| {
                let ml = Arc::clone(&ml);
                let cred = cred.to_string();
                async move { ml.fetch_model_list(&cred).await }
            })
            .await;
        println!(" done");
        ctx.fetched_models = result.into_models();
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

    // Create initial master agent if it does not exist yet.
    // Failure here is non-fatal: the wizard config (models + credentials) is
    // already written and should not be rolled back.
    let agents_dir = config_dir().parent().unwrap().join("agents");
    if let Err(e) = ensure_master_agent(&config_dir(), &agents_dir) {
        eprintln!("[WARNING] Failed to create master agent: {}", e);
    }

    Ok(Some(WizardOutput {
        provider_id: out_provider_id,
        credential: out_credential,
        selected_models: out_selected_models,
    }))
}

#[cfg(test)]
mod tests;
