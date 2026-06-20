//! Interactive chat REPL via the terminal channel.
//!
//! Creates a [`TerminalPlugin`], registers it with a [`Gateway`] configured
//! with a [`ProcessorRegistry`], [`SlashDispatcher`], and
//! [`SessionMessageHandler`], and runs a read-eval-print loop that routes
//! user input through the full inbound/outbound message pipeline.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::providers::{ConfigProvider, CredentialsProvider};
use crate::gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use crate::im::terminal::TerminalPlugin;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::fallback::{FallbackClient, ModelEntry};
use crate::llm::minimax::MiniMaxProvider;
use crate::llm::openai::OpenAIProvider;
use crate::llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use crate::llm::LLMRegistry;
use crate::processor_chain::content_normalizer::ContentNormalizer;
use crate::processor_chain::dsl_parser::DslParser;
use crate::processor_chain::raw_log_processor::{RawLogConfig, RawLogProcessor};
use crate::processor_chain::ProcessorRegistry;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::registry::HandlerRegistry;
use crate::slash::{ClearHandler, HelpHandler, NewSessionHandler, StatusHandler, StopHandler};

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
/// 2. Create a session for the given `agent_id`.
/// 3. Loop: read user input → route through gateway → print response.
pub async fn run_chat(agent_id: &str) -> anyhow::Result<()> {
    let sender_id = crate::im::terminal::current_uid();
    let (gateway, session_manager) = build_gateway().await;
    let session_id = create_session(&session_manager, agent_id, &sender_id).await?;

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. \
         Type 'quit' or 'exit' to stop.\n"
    );

    match repl_loop(&gateway, &session_id, &sender_id).await {
        ExitReason::Quit => Ok(()),
        ExitReason::Error(e) => Err(e),
    }
}

/// Build a [`Gateway`] with [`ProcessorRegistry`], [`SlashDispatcher`],
/// and [`SessionMessageHandler`] configured.
async fn build_gateway() -> (Arc<Gateway>, Arc<SessionManager>) {
    let gateway_config = GatewayConfig {
        name: "closeclaw-chat".to_string(),
        rate_limit_per_minute: 0,
        max_message_size: 16_384,
        dm_scope: DmScope::PerChannelPeer,
        ..Default::default()
    };

    let session_manager = Arc::new(SessionManager::new(
        &gateway_config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    // ── Processor Registry ─────────────────────────────────────────────
    let processor_registry = Arc::new(build_processor_registry(&gateway_config));

    let gateway = Gateway::with_processor_registry(
        gateway_config,
        Arc::clone(&session_manager),
        processor_registry,
    );

    // ── Slash Dispatcher ───────────────────────────────────────────────
    let slash_registry = Arc::new(HandlerRegistry::new());
    slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(&session_manager))));
    let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
    slash_registry.register(Arc::new(help_handler));
    slash_registry.register(Arc::new(NewSessionHandler));
    slash_registry.register(Arc::new(StopHandler));
    slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(&session_manager))));
    let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry));

    // ── Session Message Handler ────────────────────────────────────────
    let session_handler = build_session_handler(Arc::clone(&session_manager)).await;
    let gateway = if let Some(handler) = session_handler {
        gateway.with_session_handler(Arc::new(handler))
    } else {
        gateway
    };

    let gateway = Arc::new(gateway);
    gateway.set_self_ref(Arc::clone(&gateway));

    // Inject slash dispatcher after Arc wrapping (async method).
    gateway.set_slash_dispatcher(slash_dispatcher).await;

    let plugin: Arc<dyn crate::im::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    (gateway, session_manager)
}

/// Build a [`ProcessorRegistry`] with inbound [`RawLogProcessor`] and
/// [`ContentNormalizer`], outbound [`DslParser`].
fn build_processor_registry(config: &GatewayConfig) -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::default();

    // Inbound: RawLogProcessor (if raw_log_dir is configured)
    if let Some(ref dir) = config.raw_log_dir {
        let raw_log_config = RawLogConfig {
            enabled: true,
            dir: dir.clone(),
            retention_days: 7,
        };
        let processor =
            RawLogProcessor::new(raw_log_config).expect("RawLogProcessor initialization failed");
        registry.register(Arc::new(processor));
    }

    // Inbound: ContentNormalizer
    registry.register(Arc::new(ContentNormalizer::new()));

    // Outbound: DslParser
    registry.register(Arc::new(DslParser));

    registry
}

/// Initialize the LLM registry from credentials files or environment variables.
async fn init_llm_registry() -> Arc<LLMRegistry> {
    let registry = Arc::new(LLMRegistry::new());

    // Determine config directory
    let config_dir = dirs::home_dir()
        .map(|h| h.join(".closeclaw"))
        .unwrap_or_else(|| PathBuf::from(".closeclaw"));

    // Load credentials from config/credentials/ directory
    let creds_dir = config_dir.join(CredentialsProvider::config_path());
    let creds_provider = match CredentialsProvider::load_from_dir(&creds_dir) {
        Ok(cp) => cp,
        Err(e) => {
            tracing::warn!(
                "failed to load credentials from '{}': {}",
                creds_dir.display(),
                e
            );
            CredentialsProvider::default()
        }
    };

    // Register OpenAI provider
    let openai_key = creds_provider
        .get_api_key("openai")
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty());
    if let Some(api_key) = openai_key {
        let provider: Arc<dyn crate::llm::provider::Provider> =
            Arc::new(OpenAIProvider::new(api_key));
        registry.register("openai".to_string(), provider).await;
    }

    // Register Anthropic provider
    let anthropic_key = creds_provider
        .get_api_key("anthropic")
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.is_empty());
    if let Some(api_key) = anthropic_key {
        let provider: Arc<dyn crate::llm::provider::Provider> =
            Arc::new(AnthropicProvider::new(api_key));
        registry.register("anthropic".to_string(), provider).await;
    }

    // Register MiniMax provider
    let minimax_key = creds_provider
        .get_api_key("minimax")
        .or_else(|| std::env::var("MINIMAX_API_KEY").ok())
        .filter(|k| !k.is_empty());
    if let Some(api_key) = minimax_key {
        let provider: Arc<dyn crate::llm::provider::Provider> =
            Arc::new(MiniMaxProvider::new(api_key));
        registry.register("minimax".to_string(), provider).await;
    }

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

/// Build a [`SessionMessageHandler`] with LLM clients.
///
/// Returns `None` if no LLM providers are configured.
async fn build_session_handler(
    session_manager: Arc<SessionManager>,
) -> Option<crate::gateway::session_handler::SessionMessageHandler> {
    let llm_registry = init_llm_registry().await;
    let fallback_chain = build_fallback_chain();

    // Filter chain to only include providers that are registered
    let available_providers = llm_registry.list().await;
    let valid_chain: Vec<ModelEntry> = fallback_chain
        .into_iter()
        .filter(|e| available_providers.contains(&e.provider))
        .collect();

    if valid_chain.is_empty() {
        tracing::warn!("no LLM providers configured — session handler not installed");
        return None;
    }

    let fallback_client = Arc::new(FallbackClient::new(
        Arc::clone(&llm_registry),
        valid_chain.clone(),
    ));

    // Build UnifiedFallbackClient chain entries
    let cooldown = Arc::new(crate::llm::retry::CooldownManager::new());
    let unified_chain: Vec<ChainEntry> = valid_chain
        .iter()
        .filter_map(|entry| {
            let protocol = Arc::new(crate::llm::protocol::OpenAiProtocol::default());
            let interpreter_registry = crate::llm::interpreter::InterpreterRegistry::new(vec![]);
            let plugin_pipeline = crate::llm::plugin::PluginPipeline::new();
            let provider = {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(llm_registry.get(&entry.provider))
            }?;
            let client = Arc::new(crate::llm::client::UnifiedChatClient::new(
                provider,
                protocol,
                interpreter_registry,
                plugin_pipeline,
                Arc::new(crate::llm::cache_adapter::NoopCacheAdapter),
            ));
            Some(ChainEntry {
                provider_id: entry.provider.clone(),
                model_id: entry.model.clone(),
                client,
            })
        })
        .collect();

    let unified_fallback_client = Arc::new(UnifiedFallbackClient::new(unified_chain, cooldown));

    let (output_tx, _output_rx) = tokio::sync::mpsc::channel(64);
    Some(crate::gateway::session_handler::SessionMessageHandler::new(
        session_manager,
        fallback_client,
        output_tx,
        unified_fallback_client,
    ))
}

/// Create a session for the chat REPL.
async fn create_session(
    session_manager: &SessionManager,
    agent_id: &str,
    sender_id: &str,
) -> anyhow::Result<String> {
    let message = crate::gateway::Message {
        id: format!("chat-{}", chrono::Utc::now().timestamp()),
        from: sender_id.to_string(),
        to: agent_id.to_string(),
        content: String::new(),
        channel: "terminal".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: Default::default(),
        thread_id: None,
    };

    session_manager
        .find_or_create("terminal", &message, None)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create session: {}", e))
}

/// Run the read-eval-print loop.
///
/// Returns [`ExitReason::Quit`] when the user exits normally, or
/// [`ExitReason::Error`] on I/O failure.
async fn repl_loop(gateway: &Arc<Gateway>, session_id: &str, sender_id: &str) -> ExitReason {
    let stdin = io::stdin();

    loop {
        print!("> ");
        if io::stdout().flush().is_err() {
            return ExitReason::Error(anyhow::anyhow!("failed to flush stdout"));
        }

        let content = match read_user_input(&stdin) {
            InputResult::Message(c) => c,
            InputResult::Quit => {
                println!("Goodbye!");
                return ExitReason::Quit;
            }
            InputResult::Empty => continue,
            InputResult::Eof => {
                println!("\nGoodbye!");
                return ExitReason::Quit;
            }
            InputResult::IoError(e) => {
                return ExitReason::Error(anyhow::anyhow!("read error: {}", e));
            }
        };

        let result = gateway
            .handle_inbound_message(session_id, content, Some(sender_id), "terminal")
            .await;

        if result.is_none() {
            eprintln!("(no response — session handler not configured)");
        }

        // Allow the streaming response to flush to stdout.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        println!();
    }
}

/// Result of reading one user input attempt.
enum InputResult {
    /// A non-empty message ready to send.
    Message(String),
    /// User typed quit, exit, or /stop.
    Quit,
    /// Blank input — prompt again.
    Empty,
    /// EOF reached (stdin closed).
    Eof,
    /// I/O error.
    IoError(io::Error),
}

/// Read user input from stdin until a blank line (message boundary) or EOF.
fn read_user_input(stdin: &io::Stdin) -> InputResult {
    let mut lines = Vec::new();

    for line in stdin.lock().lines() {
        match line {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit") {
                    return InputResult::Quit;
                }
                if trimmed.eq_ignore_ascii_case("/stop") {
                    println!("Session stopped.");
                    return InputResult::Quit;
                }
                if trimmed.is_empty() {
                    if lines.is_empty() {
                        print!("> ");
                        let _ = io::stdout().flush();
                        continue;
                    }
                    break;
                }
                lines.push(text);
            }
            Err(e) => return InputResult::IoError(e),
        }
    }

    if lines.is_empty() {
        return InputResult::Eof;
    }

    let content = lines.join("\n");
    if content.trim().is_empty() {
        InputResult::Empty
    } else {
        InputResult::Message(content)
    }
}
