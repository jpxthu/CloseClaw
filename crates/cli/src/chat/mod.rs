//! Interactive chat REPL via the terminal channel.
//!
//! Creates an in-process Gateway instance with a registered TerminalPlugin.
//! User input flows through the inbound processor chain, is routed by the
//! Gateway, and outbound responses are rendered back to stdout.
//!
//! Startup verifies daemon reachability via the admin socket — the daemon
//! must already be running (started with `closeclaw run`).

pub mod rpc;

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

use closeclaw_gateway::{
    Gateway, GatewayConfig, HandleResult, SessionManager, SessionMessageHandler,
};
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_slash::dispatcher::SlashDispatcher;
use closeclaw_slash::handlers::CompactHandler;
use closeclaw_slash::handlers_session::{StopHandler, VerboseHandler};
use closeclaw_slash::registry::HandlerRegistry;

use crate::admin::rpc::client::{admin_socket_path, AdminClient};
use crate::llm_init;
use crate::terminal::{TerminalAdapter, TerminalPlugin};

/// Timeout for waiting for streaming LLM output.
const STREAMING_TIMEOUT_SECS: u64 = 120;

/// Why the REPL loop exited.
enum ExitReason {
    /// User typed quit or exit.
    Quit,
    /// An unrecoverable error occurred.
    Error(anyhow::Error),
}

/// Run the interactive chat REPL.
///
/// 1. Verify the daemon is reachable (admin socket ping).
/// 2. Create an in-process Gateway + TerminalPlugin.
/// 3. Initialize LLM call chain for Session/LLM integration.
/// 4. Loop: read user input → process through Gateway → render output.
async fn build_gateway(
    config_dir: &std::path::Path,
    agent_id: &str,
    _llm_registry: &Arc<closeclaw_llm::LLMRegistry>,
    fallback_client: &Arc<closeclaw_llm::unified_fallback::UnifiedFallbackClient>,
) -> anyhow::Result<(
    Arc<Gateway>,
    tokio::sync::mpsc::Receiver<(String, Vec<closeclaw_common::ContentBlock>)>,
)> {
    let mut bindings = HashMap::new();
    bindings.insert("cli".to_string(), agent_id.to_string());

    let gateway_config = GatewayConfig {
        name: "cli-chat".to_string(),
        bot_agent_bindings: bindings,
        ..Default::default()
    };
    let session_manager = Arc::new(SessionManager::new(
        &gateway_config,
        None,
        Some(config_dir.to_path_buf()),
        ReasoningLevel::default(),
    ));

    // Set LLM caller on SessionManager for ConversationSession creation.
    let llm_caller = Arc::new(closeclaw_gateway::llm_caller_impl::FallbackLlmCaller(
        Arc::clone(fallback_client),
    ));
    session_manager
        .set_llm_caller(llm_caller as Arc<dyn closeclaw_common::LlmCaller>)
        .await;

    let gateway = Arc::new(Gateway::new(gateway_config, Arc::clone(&session_manager)));
    gateway.set_self_ref(Arc::clone(&gateway));

    // ── SessionMessageHandler setup ────────────────────────────────
    let (output_tx, output_rx) =
        tokio::sync::mpsc::channel::<(String, Vec<closeclaw_common::ContentBlock>)>(64);

    let active_searcher_llm_caller = Arc::new(
        closeclaw_gateway::session_handler::ActiveSearcherLlmCaller {
            client: Arc::clone(fallback_client),
            model: String::new(),
        },
    );

    let session_handler = Arc::new(SessionMessageHandler::new(
        Arc::clone(&session_manager),
        Arc::clone(fallback_client),
        output_tx,
        active_searcher_llm_caller,
        closeclaw_common::CompactConfig::default(),
    ));
    gateway.set_session_handler(session_handler);

    // ── Slash command dispatcher setup ──────────────────────────────
    let slash_registry = Arc::new(HandlerRegistry::new());
    slash_registry.register(Arc::new(CompactHandler));
    slash_registry.register(Arc::new(StopHandler));
    let sm_query: Arc<dyn closeclaw_common::SlashSessionQuery> = session_manager.clone();
    slash_registry.register(Arc::new(VerboseHandler::new(sm_query)));
    let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry))
        as Arc<dyn closeclaw_common::SlashRouter>;
    gateway.set_slash_dispatcher(slash_dispatcher).await;

    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    Ok((gateway, output_rx))
}

pub async fn run_chat(agent_id: &str) -> anyhow::Result<()> {
    let config_dir = dirs::home_dir()
        .map(|h| h.join(".closeclaw"))
        .unwrap_or_else(|| std::path::PathBuf::from(".closeclaw"));

    // ── Step 0: daemon reachability check ──────────────────────────
    let admin_sock = admin_socket_path(&config_dir);
    let admin = AdminClient::new(admin_sock.to_string_lossy().to_string());
    if !admin.ping().await {
        anyhow::bail!(
            "daemon is not running or admin socket not found at {}\n\
             Start the daemon first: closeclaw run",
            admin_sock.display()
        );
    }

    // ── LLM initialization ────────────────────────────────────────
    let llm_registry = llm_init::init_llm_registry(&config_dir).await;
    let fallback_client = llm_init::create_fallback_client(&llm_registry).await;

    let (gateway, mut output_rx) =
        build_gateway(&config_dir, agent_id, &llm_registry, &fallback_client).await?;

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. \
         Type 'quit' or 'exit' to stop.\n"
    );

    match repl_loop(&gateway, &mut output_rx).await {
        ExitReason::Quit => Ok(()),
        ExitReason::Error(e) => Err(e),
    }
}

/// Run the read-eval-print loop through the Gateway.
///
/// Each user input is processed through the inbound processor chain,
/// routed by the Gateway, and the response is rendered to stdout.
/// When streaming output arrives via the output channel, it's written
/// to stdout immediately.
async fn route_message(
    gateway: &Arc<Gateway>,
    message: &closeclaw_common::NormalizedMessage,
    output_rx: &mut tokio::sync::mpsc::Receiver<(String, Vec<closeclaw_common::ContentBlock>)>,
) {
    // Process through Gateway inbound chain.
    let processed = gateway.process_inbound_chain(message).await;

    // Route through Gateway.
    let result = gateway
        .handle_inbound_message(processed, Some(&message.sender_id), "terminal")
        .await;

    match result {
        Some(HandleResult::LlmStarted) => {
            wait_for_streaming_completion(output_rx).await;
        }
        Some(HandleResult::SlashHandled) => {}
        Some(HandleResult::MessageQueued(text)) => {
            println!("{}", text);
        }
        Some(HandleResult::Error(msg)) => {
            eprintln!("Error: {}", msg);
        }
        Some(HandleResult::ApprovalProcessed) => {}
        None => {
            eprintln!("(message not processed — no session handler)");
        }
    }

    println!();
}

async fn repl_loop(
    gateway: &Arc<Gateway>,
    output_rx: &mut tokio::sync::mpsc::Receiver<(String, Vec<closeclaw_common::ContentBlock>)>,
) -> ExitReason {
    let adapter = TerminalAdapter::new();

    loop {
        print!("> ");
        if io::stdout().flush().is_err() {
            return ExitReason::Error(anyhow::anyhow!("failed to flush stdout"));
        }

        // Use TerminalAdapter to read and filter input (empty content → skip).
        let message = match adapter.read_input() {
            Some(m) => m,
            None => continue,
        };

        let content = message.content.trim().to_string();

        // Handle quit/exit locally before routing through Gateway.
        let lower = content.to_ascii_lowercase();
        if lower == "quit" || lower == "exit" {
            println!("Goodbye!");
            return ExitReason::Quit;
        }

        route_message(gateway, &message, output_rx).await;
    }
}

/// Wait for streaming LLM output to complete.
///
/// The SessionMessageHandler sends the final result on the output channel
/// when the streaming task finishes. This function waits for that signal.
async fn wait_for_streaming_completion(
    output_rx: &mut tokio::sync::mpsc::Receiver<(String, Vec<closeclaw_common::ContentBlock>)>,
) {
    // The output channel is closed when the streaming task completes.
    // We wait for either a message (final result) or channel close.
    match tokio::time::timeout(
        std::time::Duration::from_secs(STREAMING_TIMEOUT_SECS), // 2 minute timeout for long LLM responses
        output_rx.recv(),
    )
    .await
    {
        Ok(Some((_text, _content_blocks))) => {
            // Streaming complete — the TerminalPlugin has already written
            // the output to stdout via the Gateway's outbound pipeline.
        }
        Ok(None) => {
            // Channel closed — streaming task completed.
        }
        Err(_) => {
            eprintln!("\n(timeout waiting for LLM response)");
        }
    }
}

/// Whether the REPL should wait for streaming to complete.
///
/// Retained for test compatibility with the previous gateway-based REPL.
#[cfg(test)]
pub(crate) fn should_wait_for_streaming(
    result: Option<closeclaw_gateway::HandleResult>,
    session_key: &str,
) -> bool {
    matches!(result, Some(closeclaw_gateway::HandleResult::LlmStarted)) && !session_key.is_empty()
}
