//! Config Wizard - interactive CLI configuration flow

pub mod fetch;
pub mod types;

pub use fetch::*;
pub use types::*;

use crate::agent::config::AgentConfig;
use closeclaw_config::agents::{AgentsConfig, AgentsConfigProvider};
use closeclaw_config::providers::{
    credentials::{AnyProviderCredentials, ApiKeyCredentials},
    models::{ModelDefinition, ModelsConfigData, ProviderConfig},
};
use closeclaw_llm::{
    DeepSeekProvider, GlmProvider, MiniMaxProvider, ModelDiscovery, ModelLister,
    ProviderModelKnowledge, VolcEngineProvider,
};
use dialoguer::{Input, Select};

use std::panic;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use closeclaw_llm::model_info::ModelInfo;

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

/// Create the initial `master` agent if it does not already exist.
///
/// Writes:
/// - `~/.closeclaw/agents/master/config.json`
/// - `~/.closeclaw/config/agents.json` (appends "master" if missing)
///
/// Idempotent: skips files that already exist.
pub(crate) fn ensure_master_agent(config_dir: &Path, agents_dir: &Path) -> anyhow::Result<()> {
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

/// Merge selected models with existing models for a provider.
///
/// Strategy:
/// - Existing models not in `selected_models` are preserved (not deleted)
/// - Existing models matching by id are replaced entirely (name/enabled updated)
/// - New models from `selected_models` are appended
fn merge_models(
    existing_models: Vec<ModelDefinition>,
    selected_models: &[ModelInfo],
) -> Vec<ModelDefinition> {
    let mut merged: Vec<ModelDefinition> = existing_models;
    for selected in selected_models {
        if let Some(existing) = merged.iter_mut().find(|m| m.id == selected.id) {
            *existing = ModelDefinition {
                id: selected.id.clone(),
                name: Some(selected.name.clone()),
                enabled: Some(true),
            };
        } else {
            merged.push(ModelDefinition {
                id: selected.id.clone(),
                name: Some(selected.name.clone()),
                enabled: Some(true),
            });
        }
    }
    merged
}

/// Inherit base_url, api_key, api from an existing provider config if present.
fn inherit_provider_fields(
    existing_provider: &Option<ProviderConfig>,
) -> (Option<String>, Option<String>, Option<String>) {
    existing_provider
        .as_ref()
        .map(|ep| (ep.base_url.clone(), ep.api_key.clone(), ep.api.clone()))
        .unwrap_or((None, None, None))
}

/// Load existing models config or return a default.
fn load_or_create_models_config(models_path: &Path) -> anyhow::Result<ModelsConfigData> {
    if models_path.exists() {
        let content = std::fs::read_to_string(models_path)?;
        Ok(ModelsConfigData::from_json_str(&content)
            .unwrap_or_else(|_| ModelsConfigData::default()))
    } else {
        Ok(ModelsConfigData::default())
    }
}

/// Write provider credentials to `credentials/<provider_id>.json`.
fn write_provider_credentials(
    config_path: &Path,
    provider_id: &str,
    credential: &str,
) -> anyhow::Result<PathBuf> {
    let creds_dir = config_path.join("credentials");
    std::fs::create_dir_all(&creds_dir)?;
    let cred_file = creds_dir.join(format!("{}.json", provider_id));
    let creds = AnyProviderCredentials::ApiKey(ApiKeyCredentials {
        provider: provider_id.to_string(),
        api_key: credential.to_string(),
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
/// - `~/.closeclaw/config/models.json`   — merged provider+model config
/// - `~/.closeclaw/config/credentials/<provider_id>.json` — API key for the provider
//-
//- NOTE: Any change to `write_wizard_config` or `run_wizard` must be verified
//- against `tests/cli_config_wizard_test.py` (python3 E2E test using pexpect).
pub fn write_wizard_config_to(output: &WizardOutput, config_path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_path)?;
    let models_path = config_path.join("models.json");
    let existing = load_or_create_models_config(&models_path)?;
    let mut providers = existing.providers;
    let existing_provider = providers.get(&output.provider_id).cloned();
    providers.remove(&output.provider_id);
    let existing_models = existing_provider
        .as_ref()
        .map(|ep| ep.models.clone())
        .unwrap_or_default();
    let merged_models = merge_models(existing_models, &output.selected_models);
    let recommended_protocol = output.selected_models.first().map(|m| {
        let kb = ProviderModelKnowledge::new();
        kb.recommended_protocol(&output.provider_id, &m.id)
            .to_string()
    });
    let (base_url, _api_key, api) = inherit_provider_fields(&existing_provider);
    // api_key is never stored in models.json — credentials live in a separate
    // file referenced by credential_path. Setting it to None prevents leaking
    // secrets into the config file.
    providers.insert(
        output.provider_id.clone(),
        ProviderConfig {
            base_url,
            api_key: None,
            api,
            protocol: recommended_protocol,
            credential_path: Some(format!("credentials/{}.json", output.provider_id)),
            models: merged_models,
        },
    );
    let merged = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    <ModelsConfigData as closeclaw_config::providers::ConfigProvider>::validate(&merged)
        .map_err(|e| anyhow::anyhow!("merged config validation failed: {}", e))?;
    let json = serde_json::to_string_pretty(&merged)?;
    std::fs::write(&models_path, json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", models_path.display(), e))?;
    let cred_file =
        write_provider_credentials(config_path, &output.provider_id, &output.credential)?;
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

/// Compute a default selection string for already-configured models.
///
/// Given a list of fetched models and a set of already-configured model IDs,
/// returns a comma-separated string of 1-based indices (e.g. "1,3,5")
/// for models that exist in `configured_ids`. Returns an empty string if no
/// models are already configured.
pub(crate) fn compute_default_selection(
    models: &[ModelInfo],
    configured_ids: &std::collections::HashSet<String>,
) -> String {
    let indices: Vec<String> = models
        .iter()
        .enumerate()
        .filter(|(_, m)| configured_ids.contains(&m.id))
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    indices.join(",")
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
    let kb = ProviderModelKnowledge::new();
    let provider_display: Vec<String> = PROVIDERS
        .iter()
        .map(|p| {
            let proto = kb.recommended_protocol(p.id, "");
            let proto_str = if proto.as_str().is_empty() {
                "N/A"
            } else {
                proto.as_str()
            };
            format!(
                "{} — {} (推荐协议: {})",
                p.display_name, p.description, proto_str
            )
        })
        .collect();
    let selection = tokio::task::spawn_blocking(move || {
        Select::new()
            .with_prompt("Select a provider")
            .items(&provider_display)
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

        // Display fetched model details
        if !ctx.fetched_models.is_empty() {
            println!("\nFound {} model(s):\n", ctx.fetched_models.len());
            println!(
                " {:3}. {:35} {:>10} {:>8} Reasoning",
                "#", "Model", "Context", "MaxOut"
            );
            println!(
                "{:->4}- {:->35}- {:->10}- {:->8}- {:->9}",
                "", "", "", "", ""
            );
            for (i, m) in ctx.fetched_models.iter().enumerate() {
                let reasoning = if m.reasoning { "✅" } else { "❌" };
                println!(
                    " {:3}. {:35} {:>10} {:>8} {}",
                    i + 1,
                    m.name,
                    m.context_window,
                    m.max_tokens,
                    reasoning
                );
            }
            println!();
        }
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
                    "{:3}. {} {}{}  ctx: {}K  max: {}K  protocol: {}",
                    i + 1,
                    mark,
                    m.name,
                    reasoning_flag,
                    m.context_window / 1000,
                    m.max_tokens / 1000,
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

        // Compute default selection for already-configured models
        let configured_ids: std::collections::HashSet<String> = models
            .iter()
            .filter(|m| ctx.existing_config.contains_key(&m.id))
            .map(|m| m.id.clone())
            .collect();
        let default_selection = compute_default_selection(models, &configured_ids);

        let input: String = tokio::task::spawn_blocking(move || {
            let mut prompt = Input::new().with_prompt("Your selection");
            if !default_selection.is_empty() {
                prompt = prompt.with_initial_text(default_selection);
            }
            prompt.interact()
        })
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

    // NOTE: write_wizard_config() and ensure_master_agent() are called by
    // handle_config_setup() after Confirm — run_wizard() only builds and
    // returns the WizardOutput.

    Ok(Some(WizardOutput {
        provider_id: out_provider_id,
        credential: out_credential,
        selected_models: out_selected_models,
    }))
}

#[cfg(test)]
mod tests;
