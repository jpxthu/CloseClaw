//! Interactive wizard flow for the config wizard.

use closeclaw_llm::{
    DeepSeekProvider, GlmProvider, MiniMaxProvider, ModelDiscovery, ModelLister,
    ProviderModelKnowledge, VolcEngineProvider,
};
use dialoguer::{Input, Select};

use std::panic;
use std::sync::Arc;

use super::parse::{
    compute_default_selection, format_discovery_source_suffix, parse_model_selection,
};
use super::types::{
    ProviderInfo, ProviderType, WizardContext, WizardOutput, WizardState, PROVIDERS,
};
use super::write_config::find_models_config;

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
        let source = result.source;
        ctx.fetched_models = result.into_models();

        // Display fetched model details
        if !ctx.fetched_models.is_empty() {
            let source_suffix = format_discovery_source_suffix(source);
            println!(
                "\nFound {} model(s){}:\n",
                ctx.fetched_models.len(),
                source_suffix
            );
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
