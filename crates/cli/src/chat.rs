//! Interactive chat REPL via the terminal channel.
//!
//! Creates a [`TerminalPlugin`], registers it with a [`Gateway`] configured
//! with a [`ProcessorRegistry`], [`SlashDispatcher`], and
//! [`SessionMessageHandler`], and runs a read-eval-print loop that routes
//! user input through the full inbound/outbound message pipeline.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::terminal::TerminalPlugin;
use closeclaw_common::IMPlugin;
use closeclaw_common::LlmCaller;
use closeclaw_config::providers::{ConfigProvider, CredentialsProvider};
use closeclaw_gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use closeclaw_llm::anthropic::AnthropicProvider;
use closeclaw_llm::fallback::{FallbackClient, ModelEntry};
use closeclaw_llm::mimo::MimoProvider;
use closeclaw_llm::minimax::MiniMaxProvider;
use closeclaw_llm::openai::OpenAIProvider;
use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use closeclaw_llm::LLMRegistry;
use closeclaw_processor_chain as processor_chain;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_slash::dispatcher::SlashDispatcher;
use closeclaw_slash::registry::HandlerRegistry;
use closeclaw_slash::{ClearHandler, HelpHandler, NewSessionHandler, StatusHandler, StopHandler};

/// Why the REPL loop exited.
enum ExitReason {
    /// User typed quit/exit or /stop.
    Quit,
    /// unrecoverable error occurred.
    Error(anyhow::Error),
}

/// Run the interactive chat REPL.
///
/// 1. Build a [`Gateway`] with a [`TerminalPlugin`] registered.
/// 2. Loop: read user input → route through gateway → print response.
pub async fn run_chat(agent_id: &str) -> anyhow::Result<()> {
    let sender_id = closeclaw_platform::current_uid();
    let (gateway, _session_manager) = build_gateway(agent_id).await;

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. \
         Type 'quit' or 'exit' to stop.\n"
    );

    match repl_loop(&gateway, agent_id, &sender_id).await {
        ExitReason::Quit => Ok(()),
        ExitReason::Error(e) => Err(e),
    }
}

/// Build a [`Gateway`] with [`ProcessorRegistry`], [`SlashDispatcher`],
/// and [`SessionMessageHandler`] configured.
pub(crate) async fn build_gateway(agent_id: &str) -> (Arc<Gateway>, Arc<SessionManager>) {
    let gateway_config = GatewayConfig {
        name: format!("closeclaw-chat-{}", agent_id),
        rate_limit_per_minute: 0,
        max_message_size: 16_384,
        dm_scope: DmScope::PerChannelSender,
        ..Default::default()
    };

    let session_manager = Arc::new(SessionManager::new(
        &gateway_config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    let processor_registry = Arc::new(processor_chain::build_processor_registry(&gateway_config))
        as Arc<dyn closeclaw_common::ProcessorChain>;
    let gateway = Gateway::with_processor_registry(
        gateway_config,
        Arc::clone(&session_manager),
        processor_registry,
    );

    let slash_dispatcher = build_slash_dispatcher(Arc::clone(&session_manager));

    let gateway = attach_session_handler(gateway, Arc::clone(&session_manager)).await;
    let gateway = Arc::new(gateway);
    gateway.set_self_ref(Arc::clone(&gateway));
    gateway
        .set_slash_dispatcher(slash_dispatcher as Arc<dyn closeclaw_common::SlashRouter>)
        .await;

    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    (gateway, session_manager)
}

/// Load credentials from the config directory, falling back to defaults.
fn load_credentials_provider() -> CredentialsProvider {
    let config_dir = dirs::home_dir()
        .map(|h| h.join(".closeclaw"))
        .unwrap_or_else(|| PathBuf::from(".closeclaw"));
    let creds_dir = config_dir.join(CredentialsProvider::config_path());
    match CredentialsProvider::load_from_dir(&creds_dir) {
        Ok(cp) => cp,
        Err(e) => {
            tracing::warn!(
                "failed to load credentials from '{}': {}",
                creds_dir.display(),
                e
            );
            CredentialsProvider::default()
        }
    }
}

/// Filter the fallback chain to only include registered providers.
async fn filter_valid_chain(
    llm_registry: &LLMRegistry,
    fallback_chain: Vec<ModelEntry>,
) -> Option<Vec<ModelEntry>> {
    let available = llm_registry.list().await;
    let valid: Vec<ModelEntry> = fallback_chain
        .into_iter()
        .filter(|e| available.contains(&e.provider))
        .collect();
    if valid.is_empty() {
        tracing::warn!("no LLM providers configured — session handler not installed");
        return None;
    }
    Some(valid)
}

/// Build a [`SlashDispatcher`] with core command handlers.
fn build_slash_dispatcher(
    session_manager: Arc<SessionManager>,
) -> Arc<crate::bridge::SlashDispatcherWrapper> {
    let slash_registry = Arc::new(HandlerRegistry::new());
    slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(&session_manager))));
    let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
    slash_registry.register(Arc::new(help_handler));
    slash_registry.register(Arc::new(NewSessionHandler));
    slash_registry.register(Arc::new(StopHandler));
    slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(&session_manager))));
    Arc::new(crate::bridge::SlashDispatcherWrapper(
        SlashDispatcher::from_shared(slash_registry),
    ))
}

/// Attach a [`SessionMessageHandler`] to the gateway if LLM providers are available.
async fn attach_session_handler(gateway: Gateway, session_manager: Arc<SessionManager>) -> Gateway {
    match build_session_handler(session_manager).await {
        Some(handler) => gateway.with_session_handler(Arc::new(handler)),
        None => gateway,
    }
}

/// Register a single LLM provider if credentials are available.
async fn try_register_provider(
    registry: &LLMRegistry,
    name: &str,
    creds_provider: &CredentialsProvider,
    env_var: &str,
    create_fn: impl FnOnce(String) -> Arc<dyn closeclaw_llm::provider::Provider>,
) {
    let key = creds_provider
        .get_api_key(name)
        .or_else(|| std::env::var(env_var).ok())
        .filter(|k| !k.is_empty());
    if let Some(api_key) = key {
        let provider = create_fn(api_key);
        registry.register(name.to_string(), provider).await;
    }
}

/// Initialize the LLM registry from credentials files or environment variables.
async fn init_llm_registry() -> Arc<LLMRegistry> {
    let registry = Arc::new(LLMRegistry::new());
    let creds_provider = load_credentials_provider();

    try_register_provider(
        &registry,
        "openai",
        &creds_provider,
        "OPENAI_API_KEY",
        |k| Arc::new(OpenAIProvider::new(k)),
    )
    .await;
    try_register_provider(
        &registry,
        "anthropic",
        &creds_provider,
        "ANTHROPIC_API_KEY",
        |k| Arc::new(AnthropicProvider::new(k)),
    )
    .await;
    try_register_provider(
        &registry,
        "minimax",
        &creds_provider,
        "MINIMAX_API_KEY",
        |k| Arc::new(MiniMaxProvider::new(k)),
    )
    .await;
    try_register_provider(&registry, "mimo", &creds_provider, "MIMO_API_KEY", |k| {
        Arc::new(MimoProvider::new(k))
    })
    .await;

    registry
}

/// Build the fallback chain from environment variables or defaults.
///
/// Reads `CLOSECLAW_FALLBACK_CHAIN` (comma-separated `provider/model` pairs)
/// or falls back to a reasonable default.
fn build_fallback_chain() -> Vec<ModelEntry> {
    if let Ok(chain_str) = std::env::var("CLOSECLAW_FALLBACK_CHAIN") {
        return chain_str
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                let (provider, model) = s.split_once('/')?;
                Some(ModelEntry {
                    provider: provider.to_string(),
                    model: model.to_string(),
                })
            })
            .collect();
    }

    // Default fallback chain
    vec![
        ModelEntry {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
        },
        ModelEntry {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
        },
    ]
}

/// Build [`ChainEntry`] items by resolving providers asynchronously.
async fn build_unified_chain(
    llm_registry: &LLMRegistry,
    valid_chain: &[ModelEntry],
) -> Vec<ChainEntry> {
    let mut providers = Vec::new();
    for entry in valid_chain {
        if let Some(provider) = llm_registry.get(&entry.provider).await {
            providers.push((entry, provider));
        }
    }

    providers
        .into_iter()
        .map(|(entry, provider)| {
            let protocol: Arc<dyn closeclaw_llm::protocol::ChatProtocol> =
                if entry.provider == "minimax" {
                    Arc::new(closeclaw_llm::protocol::AnthropicProtocol::default())
                } else {
                    Arc::new(closeclaw_llm::protocol::OpenAiProtocol::default())
                };
            let interpreter_registry = closeclaw_llm::interpreter::InterpreterRegistry::new(vec![]);
            let plugin_pipeline = if entry.provider == "deepseek" {
                closeclaw_llm::plugin::PluginPipeline::new()
                    .add(Box::new(closeclaw_llm::deepseek::DeepSeekPlugin))
            } else if entry.provider == "glm" {
                closeclaw_llm::plugin::PluginPipeline::new()
                    .add(Box::new(closeclaw_llm::glm::GlmPlugin))
            } else if entry.provider == "minimax" {
                closeclaw_llm::plugin::PluginPipeline::new()
                    .add(Box::new(closeclaw_llm::minimax::MiniMaxPlugin))
            } else {
                closeclaw_llm::plugin::PluginPipeline::new()
            };
            let client = Arc::new(closeclaw_llm::client::UnifiedChatClient::new(
                provider,
                protocol,
                interpreter_registry,
                plugin_pipeline,
                closeclaw_llm::cache_adapter::for_provider(&entry.provider),
            ));
            ChainEntry {
                provider_id: entry.provider.clone(),
                model_id: entry.model.clone(),
                client,
            }
        })
        .collect()
}

/// Build a [`SessionMessageHandler`] with LLM clients.
///
/// Returns `None` if no LLM providers are configured.
async fn build_session_handler(
    session_manager: Arc<SessionManager>,
) -> Option<closeclaw_gateway::session_handler::SessionMessageHandler> {
    let llm_registry = init_llm_registry().await;
    let fallback_chain = build_fallback_chain();
    let valid_chain = filter_valid_chain(&llm_registry, fallback_chain).await?;

    let fallback_client = Arc::new(FallbackClient::new(
        Arc::clone(&llm_registry),
        valid_chain.clone(),
    ));
    let cooldown = Arc::new(closeclaw_llm::retry::CooldownManager::new());
    let unified_chain = build_unified_chain(&llm_registry, &valid_chain).await;
    let unified_fallback_client = Arc::new(UnifiedFallbackClient::new(unified_chain, cooldown));

    let (output_tx, _output_rx) = tokio::sync::mpsc::channel(64);
    let llm_caller: Arc<dyn LlmCaller> = Arc::new(
        closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(unified_fallback_client.clone()),
    );
    let fallback_llm_caller = Arc::new(
        closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
            client: unified_fallback_client,
            model: String::new(),
        },
    );
    Some(
        closeclaw_gateway::session_handler::SessionMessageHandler::new(
            session_manager,
            fallback_client,
            output_tx,
            llm_caller,
            fallback_llm_caller,
        ),
    )
}
/// Run the read-eval-print loop.
///
/// Returns [`ExitReason::Quit`] when the user exits normally, or
/// [`ExitReason::Error`] on I/O failure.
///
/// # Note on `agent_id`
///
/// `agent_id` is intentionally not used within this function. Per-agent session
/// isolation is provided by independent [`SessionManager`] instances — each agent
/// gets its own Gateway + SessionManager in [`build_gateway()`], so `agent_id`
/// does not participate in session_key calculation. The parameter is kept for API
/// clarity and forward compatibility.
async fn repl_loop(gateway: &Arc<Gateway>, agent_id: &str, _sender_id: &str) -> ExitReason {
    let _ = agent_id; // unused; isolation handled by upstream Gateway/SessionManager
    let plugin = TerminalPlugin::new();

    loop {
        print!("> ");
        if io::stdout().flush().is_err() {
            return ExitReason::Error(anyhow::anyhow!("failed to flush stdout"));
        }

        let message = match plugin.parse_inbound(&[]).await {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(e) => {
                return ExitReason::Error(anyhow::anyhow!("input error: {}", e));
            }
        };

        // Save fields before moving into InboundChainInput.
        let msg_sender_id = message.sender_id.clone();
        let msg_platform = message.platform.clone();

        let message_id = format!("cli-{}-{}", msg_sender_id, message.timestamp);

        // Run the inbound processor chain (ContentNormalizer, RawLog, etc.).
        let input = closeclaw_gateway::InboundChainInput {
            platform: message.platform,
            sender_id: message.sender_id,
            peer_id: message.peer_id,
            content: message.content,
            message_id,
            timestamp_ms: message.timestamp,
            account_id: Some(message.account_id),
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
        };
        let processed = gateway.process_inbound_chain(&input).await;

        if processed.content_blocks.is_empty() {
            continue;
        }

        let trimmed = processed.text_content().unwrap_or("").trim();

        if trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit") {
            println!("Goodbye!");
            return ExitReason::Quit;
        }
        if trimmed.eq_ignore_ascii_case("/stop") {
            println!("Session stopped.");
            println!("Goodbye!");
            return ExitReason::Quit;
        }

        let result = gateway
            .handle_inbound_message(processed, Some(&msg_sender_id), &msg_platform)
            .await;

        if result.is_none() {
            eprintln!("(no response — session handler not configured)");
        }

        // Allow the streaming response to flush to stdout.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        println!();
    }
}
