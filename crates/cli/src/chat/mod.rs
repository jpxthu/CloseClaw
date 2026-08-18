//! Interactive chat REPL via the terminal channel.
//!
//! Connects to the daemon's chat RPC server over a Unix domain socket
//! and routes user input through the daemon's full inbound/outbound
//! message pipeline via RPC.

pub mod rpc;

use std::io::{self, Write};

use crate::chat::rpc::client::{chat_socket_path, ChatRpcClient};
use crate::chat::rpc::ChatResponse;
use crate::terminal::TerminalAdapter;

/// Why the REPL loop exited.
enum ExitReason {
    /// User typed quit or exit.
    Quit,
    /// unrecoverable error occurred.
    Error(anyhow::Error),
}

/// Run the interactive chat REPL.
///
/// 1. Connect to daemon via chat RPC socket.
/// 2. Loop: read user input → send via RPC → print rendered response.
pub async fn run_chat(agent_id: &str) -> anyhow::Result<()> {
    let config_dir = dirs::home_dir()
        .map(|h| h.join(".closeclaw"))
        .unwrap_or_else(|| std::path::PathBuf::from(".closeclaw"));
    let socket_path = chat_socket_path(&config_dir);
    let client = ChatRpcClient::new(&socket_path);

    // Check daemon is reachable.
    if !client.ping().await {
        anyhow::bail!(
            "daemon is not running or chat socket not found at {}",
            socket_path.display()
        );
    }

    println!("CloseClaw Chat — agent: {}", agent_id);
    println!(
        "Type your message and press Enter. Empty line to send. \
         Type 'quit' or 'exit' to stop.\n"
    );

    match repl_loop(&client, agent_id).await {
        ExitReason::Quit => Ok(()),
        ExitReason::Error(e) => Err(e),
    }
}

/// Action to take after handling an RPC response chunk.
enum ChunkAction {
    /// Continue reading more chunks.
    Continue,
    /// Break out of the streaming inner loop (e.g., Done received).
    Break,
}

/// Run the read-eval-print loop over RPC.
///
/// Returns [`ExitReason::Quit`] when the user exits normally, or
/// [`ExitReason::Error`] on I/O failure.
async fn repl_loop(client: &ChatRpcClient, agent_id: &str) -> ExitReason {
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

        // Handle quit/exit locally before sending to daemon.
        let lower = content.to_ascii_lowercase();
        if lower == "quit" || lower == "exit" {
            println!("Goodbye!");
            return ExitReason::Quit;
        }

        // Send message via RPC.
        let mut stream = match client.send_message(agent_id, &content).await {
            Ok(s) => s,
            Err(e) => {
                return ExitReason::Error(anyhow::anyhow!("RPC send failed: {}", e));
            }
        };

        // Read streaming response chunks.
        loop {
            match stream.next().await {
                Ok(Some(response)) => match handle_rpc_response_chunk(response) {
                    ChunkAction::Continue => {}
                    ChunkAction::Break => break,
                },
                Ok(None) => break,
                Err(e) => {
                    return ExitReason::Error(anyhow::anyhow!("RPC receive failed: {}", e));
                }
            }
        }

        println!();
    }
}

/// Handle a single RPC response chunk.
///
/// Returns an action indicating whether to continue reading or break the
/// streaming loop.
fn handle_rpc_response_chunk(response: ChatResponse) -> ChunkAction {
    match response {
        ChatResponse::ContentChunk { text } => {
            print!("{}", text);
            let _ = io::stdout().flush();
            ChunkAction::Continue
        }
        ChatResponse::ThinkingChunk { text } => {
            if !text.is_empty() {
                eprint!("[Thinking] ");
                eprint!("{}", text);
                eprintln!("[end of thinking]");
            }
            ChunkAction::Continue
        }
        ChatResponse::ToolUseChunk { name, input } => {
            eprintln!("(tool use: {} — {})", name, input);
            ChunkAction::Continue
        }
        ChatResponse::ToolResultChunk { name, output } => {
            eprintln!("(tool result: {} — {})", name, output);
            ChunkAction::Continue
        }
        ChatResponse::SessionStarted { session_key } => {
            eprintln!("[session: {}]", session_key);
            ChunkAction::Continue
        }
        ChatResponse::Error { message } => {
            eprintln!("Error: {}", message);
            ChunkAction::Continue
        }
        ChatResponse::Done => ChunkAction::Break,
        ChatResponse::Pong => ChunkAction::Continue,
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
