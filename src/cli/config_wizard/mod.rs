//! Config Wizard - interactive CLI configuration flow

pub mod types;

pub use types::*;

use crate::config::providers::{
    credentials::{AnyProviderCredentials, ApiKeyCredentials},
    models::{ModelDefinition, ModelsConfigData, ProviderConfig},
};
use crate::llm::{
    DeepSeekProvider, GlmProvider, LLMProvider, MiniMaxProvider, ModelInfo, ProviderModelKnowledge,
    VolcEngineProvider,
};
use dialoguer::{Input, Select};

use std::panic;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
pub fn write_wizard_config(output: &WizardOutput) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;

    // ── Load or create models.json ─────────────────────────────────────────────
    let models_path = dir.join("models.json");
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

    // ── Merge: preserve all existing providers, replace selected provider ───────
    let mut providers = existing.providers;
    // Remove entries for the selected provider so we can replace them
    providers.remove(&output.provider_id);

    let new_provider_config = ProviderConfig {
        base_url: None,
        api_key: None,
        api: None,
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
    let creds_dir = dir.join("credentials");
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

/// Locate the models.json config file, if it exists.
fn find_models_config() -> Option<std::path::PathBuf> {
    let possible = ["config/models.json", "configs/models.json"];
    possible
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

/// Build an `Arc<dyn LLMProvider>` from the selected `ProviderInfo`.
fn build_provider(info: &ProviderInfo, credential: &str) -> Arc<dyn LLMProvider> {
    match info.provider_type {
        ProviderType::Minimax => Arc::new(MiniMaxProvider::new(credential.to_string())),
        ProviderType::Glm => Arc::new(GlmProvider::new(credential.to_string())),
        ProviderType::Volcengine => Arc::new(VolcEngineProvider::new(credential.to_string())),
        ProviderType::Deepseek => Arc::new(DeepSeekProvider::new(credential.to_string())),
    }
}

/// Fetch model list with a 10-second timeout.
/// On timeout or error, falls back to `ProviderModelKnowledge`.
fn fetch_models_with_fallback(provider: &Arc<dyn LLMProvider>, credential: &str) -> Vec<ModelInfo> {
    // Show spinner while fetching
    print!("Fetching models from provider...");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
    let result = rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            provider.fetch_model_list(credential),
        )
        .await
    });

    match result {
        Ok(Ok(model_infos)) => {
            println!(" done");
            model_infos
        }
        Ok(Err(err)) => {
            println!(
                "\n[Warning] API fetch failed ({}), falling back to knowledge base",
                err
            );
            knowledge_fallback(provider.name())
        }
        Err(_) => {
            println!("\n[Warning] API fetch timed out (10s), falling back to knowledge base");
            knowledge_fallback(provider.name())
        }
    }
}

/// Return model list from `ProviderModelKnowledge` for the given provider name.
pub fn knowledge_fallback(provider_name: &str) -> Vec<ModelInfo> {
    let kb = ProviderModelKnowledge::new();
    let model_ids = kb.all_models(provider_name);
    model_ids
        .into_iter()
        .map(|id| {
            let params = kb.find(provider_name, id).unwrap();
            ModelInfo {
                id: id.to_string(),
                name: id.to_string(),
                context_window: params.context_window,
                max_tokens: params.max_tokens,
                default_temperature: Some(params.default_temperature),
                reasoning: params.reasoning,
                input_types: params.input_types,
            }
        })
        .collect()
}

/// Run the interactive config wizard.
///
/// Returns `Ok(Some(output))` on success, `Ok(None)` on clean Ctrl+C exit,
/// or an error for invalid input / unexpected failures.
pub fn run_wizard() -> anyhow::Result<Option<WizardOutput>> {
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
    let selection = Select::new()
        .with_prompt("Select a provider")
        .items(&PROVIDERS.iter().map(|p| p.display_name).collect::<Vec<_>>())
        .default(0)
        .interact()?;
    ctx.selected_provider = Some(PROVIDERS[selection].clone());

    // ── InputCredential ────────────────────────────────────────────────
    loop {
        let input = dialoguer::Password::new()
            .with_prompt(format!(
                "Enter API token for {}",
                ctx.selected_provider.as_ref().unwrap().display_name
            ))
            .interact()?;
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
        ctx.provider = Some(build_provider(info, cred));
        let provider = ctx.provider.as_ref().unwrap();

        let models = fetch_models_with_fallback(provider, cred);
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
                format!("{:3}. {} {}{}", i + 1, mark, m.name, reasoning_flag)
            })
            .collect();

        println!("\n=== Select Models ===");
        println!("Enter model numbers (e.g. 1 3 5, or 1-3,5,7), or 'all' to select everything.");
        println!("Already configured models are marked with [*].\n");
        for item in &display_items {
            println!("{}", item);
        }
        println!();

        let input: String = Input::new().with_prompt("Your selection").interact()?;

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
        let confirm: String = Input::new()
            .with_prompt("Confirm? (yes/no)")
            .default("yes".into())
            .interact()?;

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
