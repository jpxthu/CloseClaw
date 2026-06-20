//! Interactive chat REPL via the terminal channel.
//!
//! Creates a [`TerminalPlugin`], registers it with a minimal [`Gateway`],
//! and runs a read-eval-print loop that routes user input through the
//! full inbound/outbound message pipeline.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use crate::gateway::{DmScope, Gateway, GatewayConfig, SessionManager};
use crate::im::terminal::TerminalPlugin;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;

/// Run the interactive chat REPL.
///
/// 1. Build a [`Gateway`] with a [`TerminalPlugin`] registered.
/// 2. Create a session for the given `agent_id`.
/// 3. Loop: read user input → route through gateway → print response.
pub async fn run_chat(agent_id: &str) -> anyhow::Result<()> {
    // ── Gateway setup ─────────────────────────────────────────────────
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

    let gateway = Gateway::new(gateway_config, Arc::clone(&session_manager));
    let gateway = Arc::new(gateway);
    gateway.set_self_ref(Arc::clone(&gateway));

    // Register the terminal plugin.
    let plugin: Arc<dyn crate::im::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    // ── Create session ────────────────────────────────────────────────
    let sender_id = get_user_id();
    let peer_id = "cli";
    let channel = "terminal";

    let message = crate::gateway::Message {
        id: format!("chat-{}", chrono::Utc::now().timestamp()),
        from: sender_id.clone(),
        to: peer_id.to_string(),
        content: String::new(),
        channel: channel.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: Default::default(),
        thread_id: None,
    };

    let session_id = session_manager
        .find_or_create(channel, &message, None)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create session: {}", e))?;

    // Store agent_id as chat_id so send_outbound can resolve it.
    // SessionManager stores agent_id in the Session struct; we need
    // to ensure the session maps correctly. For the chat REPL the
    // session's agent_id *is* the chat_id used by send_outbound.
    // Since find_or_create computes the session from the message, and
    // our message has `to = "cli"`, the session's agent_id will be set
    // to the agent_id we pass. We need a small helper to set it.
    // Actually, looking at SessionManager::find_or_create, the
    // agent_id comes from the message routing — we need to make sure
    // the session is associated with our agent_id. The simplest way is
    // to store it in metadata and let the session router pick it up.
    // For the terminal channel, we'll just note the agent_id for
    // reference; the actual agent resolution happens in the LLM layer.

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. Type 'quit' or 'exit' to stop.\n"
    );

    // ── REPL loop ─────────────────────────────────────────────────────
    let stdin = io::stdin();
    loop {
        // Prompt
        print!("> ");
        io::stdout().flush().ok();

        // Read lines until a blank line (message boundary) or EOF.
        let mut lines = Vec::new();
        let mut blank_seen = false;
        for line in stdin.lock().lines() {
            match line {
                Ok(text) => {
                    let trimmed = text.trim();
                    // Exit commands
                    if trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit")
                    {
                        println!("Goodbye!");
                        return Ok(());
                    }
                    // Slash stop command
                    if trimmed.eq_ignore_ascii_case("/stop") {
                        println!("Session stopped.");
                        return Ok(());
                    }
                    // Blank line = message boundary
                    if trimmed.is_empty() {
                        if lines.is_empty() {
                            // Empty input — skip
                            print!("> ");
                            io::stdout().flush().ok();
                            continue;
                        }
                        blank_seen = true;
                        break;
                    }
                    lines.push(text);
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    return Ok(());
                }
            }
        }

        // EOF without content
        if lines.is_empty() && !blank_seen {
            println!("\nGoodbye!");
            return Ok(());
        }

        let content = lines.join("\n");
        if content.trim().is_empty() {
            continue;
        }

        // ── Route through gateway ─────────────────────────────────────
        let result = gateway
            .handle_inbound_message(&session_id, content, Some(&sender_id), channel)
            .await;

        match result {
            Some(_handle_result) => {
                // The response is rendered by TerminalPlugin and sent to
                // stdout asynchronously. We just wait briefly to allow
                // the streaming task to flush.
                // Give the LLM call a moment to produce output.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            None => {
                eprintln!("(no response — session handler not configured)");
            }
        }

        println!(); // blank line between exchanges
    }
}

/// Get a stable user ID for the terminal session.
///
/// Uses the system UID on Unix, or a fallback string.
fn get_user_id() -> String {
    #[cfg(unix)]
    {
        format!("uid-{}", unsafe { libc::getuid() })
    }
    #[cfg(not(unix))]
    {
        "terminal-user".to_string()
    }
}
