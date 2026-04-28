//! `closeclaw chat` CLI command
//!
//! Connects to the chat TCP server (default: 127.0.0.1:18889) and provides
//! either a REPL interface or single-shot message mode.

use anyhow::Context;
use clap::Parser;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::info;

const DEFAULT_CHAT_ADDR: &str = "127.0.0.1:18889";
/// Default agent for chat sessions (when client doesn't specify).
const DEFAULT_AGENT_ID: &str = "guide";

#[derive(Parser, Debug)]
#[command(name = "chat")]
#[command(about = "Chat with the CloseClaw agent via the TCP protocol")]
pub struct ChatCommand {
    /// Send a single message and print the response, then exit
    #[arg(short, long)]
    message: Option<String>,

    /// TCP address of the chat server
    #[arg(long, default_value = DEFAULT_CHAT_ADDR)]
    addr: String,

    /// Agent ID to use when starting a session (default: guide, or CLOSEWCLAW_DEFAULT_AGENT env)
    #[arg(long, default_value = DEFAULT_AGENT_ID)]
    agent_id: String,
}

// --- Entry point & connection helpers ---

impl ChatCommand {
    /// Run the chat CLI — either single-shot or REPL mode
    pub async fn run(&self) -> anyhow::Result<()> {
        let agent_id = self.resolve_agent_id();
        let addr: SocketAddr = self.addr.parse().with_context(|| {
            format!(
                "invalid address '{}' (expected format: 127.0.0.1:18889)",
                self.addr
            )
        })?;

        if let Some(ref msg) = self.message {
            self.run_single(addr, &agent_id, msg).await
        } else {
            self.run_repl(addr, &agent_id).await
        }
    }

    fn resolve_agent_id(&self) -> String {
        if self.agent_id == DEFAULT_AGENT_ID {
            std::env::var("CLOSEWCLAW_DEFAULT_AGENT")
                .unwrap_or_else(|_| DEFAULT_AGENT_ID.to_string())
        } else {
            self.agent_id.clone()
        }
    }
}

// --- Session lifecycle helpers ---

impl ChatCommand {
    /// Connect and start a chat session, returning the connected stream.
    /// Connect and start a chat session. Returns (stream, session_id).
    async fn start_session(
        addr: SocketAddr,
        agent_id: &str,
    ) -> anyhow::Result<(TcpStream, String)> {
        let mut stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("cannot connect to {} — is the daemon running?", addr))?;

        let request_id = uuid::Uuid::new_v4().to_string();
        let start_json = serde_json::json!({
            "type": "chat.start",
            "agent_id": agent_id,
            "id": request_id,
        });
        send_json_line(&mut stream, &start_json).await?;

        let resp = read_line(&mut stream).await?;
        let resp_val: serde_json::Value = serde_json::from_str(&resp)?;
        if resp_val.get("type").and_then(|v| v.as_str()) != Some("chat.started") {
            anyhow::bail!("unexpected response to chat.start: {}", resp);
        }
        let session_id = resp_val
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok((stream, session_id))
    }

    /// Send a chat.message and return its ID.
    async fn send_user_message(stream: &mut TcpStream, content: &str) -> anyhow::Result<String> {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let msg_json = serde_json::json!({
            "type": "chat.message",
            "content": content,
            "id": &msg_id,
        });
        send_json_line(stream, &msg_json).await?;
        Ok(msg_id)
    }

    /// Send a chat.stop message.
    async fn send_stop(writer: &mut (impl AsyncWriteExt + Unpin)) -> anyhow::Result<()> {
        let stop_json = serde_json::json!({
            "type": "chat.stop",
            "id": uuid::Uuid::new_v4().to_string(),
        });
        send_json_line(writer, &stop_json).await
    }
}

// --- Single-shot mode ---

impl ChatCommand {
    /// Connect, send one message, print response, disconnect.
    async fn run_single(
        &self,
        addr: SocketAddr,
        agent_id: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let (mut stream, _) = Self::start_session(addr, agent_id).await?;
        Self::send_user_message(&mut stream, message).await?;
        Self::handle_single_response(&mut stream).await?;
        Self::send_stop(&mut stream).await?;
        Ok(())
    }

    /// Read responses until chat.response.done.
    async fn handle_single_response(stream: &mut TcpStream) -> anyhow::Result<()> {
        loop {
            let resp = read_line(stream).await?;
            let resp_val: serde_json::Value = serde_json::from_str(&resp)?;
            let msg_type = resp_val.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match msg_type {
                "chat.response" => {
                    if let Some(content) = resp_val.get("content").and_then(|v| v.as_str()) {
                        print!("{}", content);
                    }
                }
                "chat.response.done" => {
                    println!();
                    return Ok(());
                }
                "chat.error" => {
                    let err_msg = resp_val
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    anyhow::bail!("server error: {}", err_msg);
                }
                _ => {}
            }
        }
    }
}

// --- REPL mode ---

impl ChatCommand {
    /// Connect and run an interactive REPL.
    async fn run_repl(&self, addr: SocketAddr, agent_id: &str) -> anyhow::Result<()> {
        let (stream, session_id) = Self::start_session(addr, agent_id).await?;
        info!(session_id = %session_id, "chat session started");
        println!("Connected to CloseClaw chat (session: {})", session_id);
        println!("Type your messages and press Enter to send. Type 'quit' or 'exit' to stop.\n");

        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = BufReader::new(reader).lines();
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        Self::repl_loop(&mut lines, &mut stdin, &mut writer).await?;
        Self::send_stop(&mut writer).await?;
        Ok(())
    }

    /// Main REPL event loop.
    async fn repl_loop(
        server_lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<TcpStream>>>,
        stdin_lines: &mut tokio::io::Lines<BufReader<tokio::io::Stdin>>,
        writer: &mut tokio::io::WriteHalf<TcpStream>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                server_line = server_lines.next_line() => {
                    if Self::handle_server_message(server_line?).await? {
                        break;
                    }
                }
                stdin_line = stdin_lines.next_line() => {
                    if Self::handle_stdin_line(stdin_line?, writer).await? {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

// --- REPL handlers ---

impl ChatCommand {
    /// Handle a server message. Returns Ok(true) if the loop should break.
    async fn handle_server_message(line: Option<String>) -> anyhow::Result<bool> {
        match line {
            Some(line) => {
                let resp_val: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("[raw] {}", line);
                        return Ok(false);
                    }
                };
                let msg_type = resp_val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match msg_type {
                    "chat.response" => {
                        if let Some(content) = resp_val.get("content").and_then(|v| v.as_str()) {
                            print!("{}", content);
                        }
                    }
                    "chat.response.done" => {
                        println!();
                    }
                    "chat.error" => {
                        let err = resp_val
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        eprintln!("\n[error] {}\n", err);
                    }
                    _ => {
                        eprintln!("[{}] {}", msg_type, line);
                    }
                }
                Ok(false)
            }
            None => {
                println!("\nServer closed the connection.");
                Ok(true)
            }
        }
    }
}

// --- REPL stdin handler ---

impl ChatCommand {
    /// Handle stdin input. Returns Ok(true) if the loop should break.
    async fn handle_stdin_line(
        line: Option<String>,
        writer: &mut tokio::io::WriteHalf<TcpStream>,
    ) -> anyhow::Result<bool> {
        match line {
            Some(input) => {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    return Ok(false);
                }
                if trimmed == "quit" || trimmed == "exit" {
                    println!("Goodbye!");
                    return Ok(true);
                }
                let msg_json = serde_json::json!({
                    "type": "chat.message",
                    "content": trimmed,
                    "id": uuid::Uuid::new_v4().to_string(),
                });
                send_json_line(writer, &msg_json).await?;
                Ok(false)
            }
            None => Ok(true),
        }
    }
}

// --- Free functions ---

/// Send a JSON value as a newline-terminated line.
async fn send_json_line(
    stream: &mut (impl AsyncWriteExt + Unpin),
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    let line = serde_json::to_string(value).unwrap();
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
}

/// Read a single newline-delimited JSON line from a stream
async fn read_line(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut buf = tokio::io::BufReader::new(stream).lines();
    let line = buf
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("server closed connection"))?;
    Ok(line)
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
