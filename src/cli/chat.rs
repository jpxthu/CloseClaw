//! `closeclaw chat` CLI command
//!
//! Connects to the chat TCP server (default: 127.0.0.1:18889) and provides
//! either a REPL interface or single-shot message mode.

use anyhow::Context;
use clap::Parser;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{error, info};

const DEFAULT_CHAT_ADDR: &str = "127.0.0.1:18889";

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

    /// Agent ID to use when starting a session
    #[arg(long, default_value = "default")]
    agent_id: String,
}

impl ChatCommand {
    /// Run the chat CLI — either single-shot or REPL mode
    pub async fn run(&self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse().with_context(|| {
            format!("invalid address '{}' (expected format: 127.0.0.1:18889)", self.addr)
        })?;

        if let Some(ref msg) = self.message {
            self.run_single(addr, msg).await
        } else {
            self.run_repl(addr).await
        }
    }

    /// Connect to the server, start a session, optionally send one message, print response
    async fn run_single(&self, addr: SocketAddr, message: &str) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("cannot connect to {} — is the daemon running?", addr))?;

        let request_id = uuid::Uuid::new_v4().to_string();

        // Send chat.start
        let start_json = serde_json::json!({
            "type": "chat.start",
            "agent_id": self.agent_id,
            "id": request_id.clone(),
        });
        let line = serde_json::to_string(&start_json).unwrap();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        // Wait for chat.started
        let resp = read_line(&mut stream).await?;
        let resp_val: serde_json::Value = serde_json::from_str(&resp)?;
        if resp_val.get("type").and_then(|v| v.as_str()) != Some("chat.started") {
            anyhow::bail!("unexpected response to chat.start: {}", resp);
        }
        let session_id = resp_val.get("session_id").and_then(|v| v.as_str()).unwrap_or("");

        // Send chat.message
        let msg_id = uuid::Uuid::new_v4().to_string();
        let msg_json = serde_json::json!({
            "type": "chat.message",
            "content": message,
            "id": msg_id,
        });
        let line = serde_json::to_string(&msg_json).unwrap();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        // Read responses until chat.response.done
        loop {
            let resp = read_line(&mut stream).await?;
            let resp_val: serde_json::Value = serde_json::from_str(&resp)?;
            let msg_type = resp_val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type == "chat.response" {
                if let Some(content) = resp_val.get("content").and_then(|v| v.as_str()) {
                    print!("{}", content);
                }
            } else if msg_type == "chat.response.done" {
                println!();
                break;
            } else if msg_type == "chat.error" {
                let err_msg = resp_val.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
                anyhow::bail!("server error: {}", err_msg);
            }
        }

        // Send chat.stop
        let stop_id = uuid::Uuid::new_v4().to_string();
        let stop_json = serde_json::json!({
            "type": "chat.stop",
            "id": stop_id,
        });
        let line = serde_json::to_string(&stop_json).unwrap();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        Ok(())
    }

    /// Connect to the server and run an interactive REPL
    async fn run_repl(&self, addr: SocketAddr) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("cannot connect to {} — is the daemon running?", addr))?;

        let request_id = uuid::Uuid::new_v4().to_string();

        // Send chat.start
        let start_json = serde_json::json!({
            "type": "chat.start",
            "agent_id": self.agent_id,
            "id": request_id,
        });
        let line = serde_json::to_string(&start_json).unwrap();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        // Wait for chat.started
        let resp = read_line(&mut stream).await?;
        let resp_val: serde_json::Value = serde_json::from_str(&resp)?;
        if resp_val.get("type").and_then(|v| v.as_str()) != Some("chat.started") {
            anyhow::bail!("unexpected response to chat.start: {}", resp);
        }
        let session_id = resp_val.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
        info!(session_id = %session_id, "chat session started");

        println!("Connected to CloseClaw chat (session: {})", session_id);
        println!("Type your messages and press Enter to send. Type 'quit' or 'exit' to stop.\n");

        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = BufReader::new(reader).lines();
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        loop {
            // Wait for either stdin or server message
            tokio::select! {
                // Read from server
                server_line = lines.next_line() => {
                    match server_line {
                        Ok(Some(line)) => {
                            let resp_val: serde_json::Value = match serde_json::from_str(&line) {
                                Ok(v) => v,
                                Err(_) => {
                                    eprintln!("[raw] {}", line);
                                    continue;
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
                                    let err = resp_val.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    eprintln!("\n[error] {}\n", err);
                                }
                                _ => {
                                    eprintln!("[{}] {}", msg_type, line);
                                }
                            }
                        }
                        Ok(None) => {
                            println!("\nServer closed the connection.");
                            break;
                        }
                        Err(e) => {
                            error!(error = %e, "error reading from server");
                            break;
                        }
                    }
                }
                // Read from stdin
                stdin_line = stdin.next_line() => {
                    match stdin_line {
                        Ok(Some(input)) => {
                            let trimmed = input.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if trimmed == "quit" || trimmed == "exit" {
                                println!("Goodbye!");
                                break;
                            }
                            // Send chat.message
                            let msg_id = uuid::Uuid::new_v4().to_string();
                            let msg_json = serde_json::json!({
                                "type": "chat.message",
                                "content": trimmed,
                                "id": msg_id,
                            });
                            let line = serde_json::to_string(&msg_json).unwrap();
                            writer.write_all(line.as_bytes()).await?;
                            writer.write_all(b"\n").await?;
                            writer.flush().await?;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            error!(error = %e, "error reading stdin");
                            break;
                        }
                    }
                }
            }
        }

        // Send chat.stop
        let stop_id = uuid::Uuid::new_v4().to_string();
        let stop_json = serde_json::json!({
            "type": "chat.stop",
            "id": stop_id,
        });
        let line = serde_json::to_string(&stop_json).unwrap();
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        Ok(())
    }
}

/// Read a single newline-delimited JSON line from a stream
async fn read_line(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut buf = tokio::io::BufReader::new(stream).lines();
    let line = buf.next_line().await?.ok_or_else(|| anyhow::anyhow!("server closed connection"))?;
    Ok(line)
}
