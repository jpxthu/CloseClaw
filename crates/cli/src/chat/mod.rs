//! Interactive chat REPL via the terminal channel.
//!
//! Creates an in-process Gateway instance with a registered TerminalPlugin.
//! User input flows through the inbound processor chain, is routed by the
//! Gateway, and outbound responses are rendered back to stdout.
//!
//! Startup verifies daemon reachability via the admin socket — the daemon
//! must already be running (started with `closeclaw run`).

pub mod rpc;

use std::io::{self, Write};
use std::sync::Arc;

use closeclaw_gateway::{Gateway, GatewayConfig, HandleResult, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;

use crate::admin::rpc::client::{admin_socket_path, AdminClient};
use crate::terminal::{TerminalAdapter, TerminalPlugin};

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
/// 3. Loop: read user input → process through Gateway → render output.
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

    // ── Gateway + TerminalPlugin setup ─────────────────────────────
    let gateway_config = GatewayConfig {
        name: "cli-chat".to_string(),
        ..Default::default()
    };
    let session_manager = Arc::new(SessionManager::new(
        &gateway_config,
        None,
        Some(config_dir.clone()),
        ReasoningLevel::default(),
    ));
    let gateway = Arc::new(Gateway::new(gateway_config, Arc::clone(&session_manager)));
    gateway.set_self_ref(Arc::clone(&gateway));

    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. \
         Type 'quit' or 'exit' to stop.\n"
    );

    match repl_loop(&gateway, agent_id).await {
        ExitReason::Quit => Ok(()),
        ExitReason::Error(e) => Err(e),
    }
}

/// Run the read-eval-print loop through the Gateway.
///
/// Each user input is processed through the inbound processor chain,
/// routed by the Gateway, and the response is rendered to stdout.
async fn repl_loop(gateway: &Arc<Gateway>, _agent_id: &str) -> ExitReason {
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

        // Process through Gateway inbound chain.
        let processed = gateway.process_inbound_chain(&message).await;

        // Route through Gateway.
        let result = gateway
            .handle_inbound_message(processed, Some(&message.sender_id), "terminal")
            .await;

        match result {
            Some(HandleResult::LlmStarted) => {
                // Streaming output is rendered by the TerminalPlugin
                // through the Gateway's outbound pipeline. Wait briefly
                // for the streaming task to start.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
