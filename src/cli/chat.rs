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

/// Build a [`Gateway`] with a [`TerminalPlugin`] registered.
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

    let gateway = Gateway::new(gateway_config, Arc::clone(&session_manager));
    let gateway = Arc::new(gateway);
    gateway.set_self_ref(Arc::clone(&gateway));

    let plugin: Arc<dyn crate::im::IMPlugin> = Arc::new(TerminalPlugin::new());
    gateway.register_plugin(plugin).await;

    (gateway, session_manager)
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
