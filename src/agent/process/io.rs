//! IO readers for agent process stdout/stderr

use super::*;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Spawn a background task to read stdout from a process
pub async fn spawn_output_reader(
    handle: AgentProcessHandle,
) -> Result<tokio::sync::mpsc::Receiver<ProcessMessage>, ProcessError> {
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let agent_id = handle.agent_id.clone();
    let mut child = handle.child.write().await;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match AgentProcess::parse_message(&line) {
                    Ok(msg) => {
                        debug!(agent_id = %agent_id, msg_type = %msg.msg_type, "received message from agent");
                        if tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(agent_id = %agent_id, error = %e, "failed to parse agent message");
                    }
                }
            }
            debug!(agent_id = %agent_id, "output reader task ended");
        });
    }

    Ok(rx)
}

/// Spawn a background task to read stderr from a process
pub async fn spawn_error_reader(
    handle: AgentProcessHandle,
) -> Result<tokio::sync::mpsc::Receiver<String>, ProcessError> {
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let agent_id = handle.agent_id.clone();
    let mut child = handle.child.write().await;

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if tx.send(line).await.is_err() {
                    break;
                }
            }
            debug!(agent_id = %agent_id, "error reader task ended");
        });
    }

    Ok(rx)
}
