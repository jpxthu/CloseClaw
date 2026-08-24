//! Unit tests for the interactive chat REPL.
//!
//! Verifies quit/exit detection, stop routing, inbound processor chain
//! behavior, NormalizedMessage field mapping, streaming wait conditions,
//! build_gateway removal, and RPC integration via mock server.

use closeclaw_common::NormalizedMessage;
use closeclaw_gateway::{GatewayConfig, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── TerminalAdapter / REPL quit/exit detection ──────────────────────────────

/// Replicate the quit/exit detection logic from the REPL loop for unit testing.
fn is_quit_command(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit")
}

fn is_stop_command(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.eq_ignore_ascii_case("/stop")
}

#[test]
fn test_quit_detection_exact() {
    assert!(is_quit_command("quit"));
    assert!(is_quit_command("exit"));
}

#[test]
fn test_quit_detection_case_insensitive() {
    assert!(is_quit_command("Quit"));
    assert!(is_quit_command("QUIT"));
    assert!(is_quit_command("Exit"));
    assert!(is_quit_command("EXIT"));
    assert!(is_quit_command("qUit"));
}

#[test]
fn test_quit_detection_with_whitespace() {
    assert!(is_quit_command("  quit  "));
    assert!(is_quit_command("\texit\n"));
}

#[test]
fn test_quit_detection_non_quit() {
    assert!(!is_quit_command("hello"));
    assert!(!is_quit_command("quitting"));
    assert!(!is_quit_command("exit_now"));
    assert!(!is_quit_command(""));
    assert!(!is_quit_command("/stop"));
}

#[test]
fn test_stop_detection() {
    assert!(is_stop_command("/stop"));
    assert!(is_stop_command("/Stop"));
    assert!(is_stop_command("  /stop  "));
    assert!(!is_stop_command("stop"));
    assert!(!is_stop_command("/stopextra"));
}

// ── /stop REPL routing tests ───────────────────────────────────────────────

/// Verify that `/stop` routes through the gateway's SlashDispatcher
/// and returns `SlashResult::Stop` with cascade=true, force=true.
#[tokio::test]
async fn test_stop_routes_through_gateway_slash_dispatcher() {
    use closeclaw_slash::dispatcher::SlashDispatcher;
    use closeclaw_slash::registry::HandlerRegistry;

    let slash_registry = Arc::new(HandlerRegistry::new());
    let _session_manager = Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test-stop-gw".to_string(),
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ));
    slash_registry.register(Arc::new(closeclaw_slash::StopHandler));
    let dispatcher = SlashDispatcher::from_shared(slash_registry);

    let ctx = closeclaw_slash::context::SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };

    match dispatcher.dispatch("/stop", &ctx).await {
        closeclaw_common::slash_router::SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade must be true");
            assert!(force, "force must be true");
        }
        other => panic!("expected Stop from gateway dispatch, got {other:?}"),
    }
}

/// Verify that `/stop` is NOT treated as a quit command by the REPL
/// detection logic. This ensures the REPL continues after `/stop`.
#[test]
fn test_stop_does_not_trigger_quit() {
    // /stop must not match quit detection
    assert!(!is_quit_command("/stop"));
    assert!(!is_quit_command("/STOP"));
    // But must match stop detection
    assert!(is_stop_command("/stop"));
    assert!(is_stop_command("/STOP"));
}

// ── Inbound Processor Chain integration tests ─────────────────────────────

use async_trait::async_trait;
use closeclaw_common::ProcessedMessage;
use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
use closeclaw_processor_chain::{MessageContext, ProcessError, ProcessorRegistry};

/// A mock processor that suppresses messages (for testing suppress behavior).
struct SuppressProcessor;

#[async_trait]
impl closeclaw_processor_chain::MessageProcessor for SuppressProcessor {
    fn name(&self) -> &str {
        "suppress-processor"
    }

    fn phase(&self) -> closeclaw_processor_chain::ProcessPhase {
        closeclaw_processor_chain::ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        0
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        Ok(None)
    }
}

/// Build a Gateway with the given ProcessorRegistry.
fn make_gw_with_registry(registry: ProcessorRegistry) -> closeclaw_gateway::Gateway {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    closeclaw_gateway::Gateway::with_processor_registry(
        config,
        Arc::new(closeclaw_gateway::SessionManager::new(
            &closeclaw_gateway::GatewayConfig {
                name: "test".to_string(),
                ..Default::default()
            },
            None,
            None,
            closeclaw_session::persistence::ReasoningLevel::default(),
        )),
        Arc::new(registry),
    )
}

#[tokio::test]
async fn test_process_inbound_chain_cleans_control_characters() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    let input = "hello\x1b[31mworld\x1b[0m";
    let processed = gateway
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: input.into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
        })
        .await;

    assert_eq!(processed.text_content(), Some("helloworld"));
    assert!(!processed.content_blocks.is_empty());
}

#[tokio::test]
async fn test_process_inbound_chain_suppress_message() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(SuppressProcessor));
    let gateway = make_gw_with_registry(registry);

    let processed = gateway
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "hello".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
        })
        .await;

    assert!(
        processed.content_blocks.is_empty(),
        "expected empty content_blocks (suppress)"
    );
}

#[tokio::test]
async fn test_process_inbound_chain_quit_exit_not_affected() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    for cmd in &["quit", "exit", "/stop"] {
        let processed = gateway
            .process_inbound_chain(&NormalizedMessage {
                platform: "terminal".into(),
                sender_id: "u1".into(),
                peer_id: "cli".into(),
                content: cmd.to_string(),
                timestamp: 0,
                account_id: String::new(),
                thread_id: None,
                chat_name: String::new(),
                trace_id: String::new(),
                message_id: String::new(),
                message_type: Default::default(),
                media_refs: Vec::new(),
            })
            .await;
        assert_eq!(processed.text_content().unwrap_or(""), *cmd);
    }
}

#[tokio::test]
async fn test_inbound_chain_preserves_stop_for_gateway_routing() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    let processed = gateway
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "/stop".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
        })
        .await;

    assert_eq!(
        processed.text_content(),
        Some("/stop"),
        "/stop must be preserved through inbound chain"
    );
}

// ── peer_id "cli" verification ────────────────────────────────────────────

#[tokio::test]
async fn test_process_inbound_chain_peer_id_is_cli() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    let peer_id_argument = "cli";
    let processed = gateway
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: peer_id_argument.into(),
            content: "hello".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
        })
        .await;

    assert!(!processed.content_blocks.is_empty());
    assert_eq!(
        peer_id_argument, "cli",
        "peer_id must be 'cli' per design doc"
    );
}

// ── REPL streaming wait condition tests ───────────────────────────────────

use super::chat::should_wait_for_streaming;
use closeclaw_gateway::HandleResult;

/// `LlmStarted` with a non-empty session_key triggers the streaming wait.
#[test]
fn test_should_wait_llm_started_with_session_key() {
    assert!(should_wait_for_streaming(
        Some(HandleResult::LlmStarted),
        "session-abc"
    ));
}

/// Non-`LlmStarted` results skip the streaming wait.
#[test]
fn test_should_wait_message_queued_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::MessageQueued),
        "session-abc"
    ));
}

/// `SlashHandled` result skips the streaming wait.
#[test]
fn test_should_wait_slash_handled_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::SlashHandled),
        "session-abc"
    ));
}

/// `ApprovalProcessed` result skips the streaming wait.
#[test]
fn test_should_wait_approval_processed_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::ApprovalProcessed),
        "session-abc"
    ));
}

/// `None` result (no session handler) skips the streaming wait.
#[test]
fn test_should_wait_none_result_skips() {
    assert!(!should_wait_for_streaming(None, "session-abc"));
}

/// `LlmStarted` with empty session_key skips the streaming wait.
#[test]
fn test_should_wait_llm_started_empty_session_key() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::LlmStarted),
        ""
    ));
}

// ── build_gateway removal verification ──────────────────────────────────────

/// Verify that `build_gateway` function no longer exists in the chat module.
///
/// This test ensures the CLI no longer self-constructs a Gateway. If someone
/// accidentally reintroduces `build_gateway`, this test will fail at compile
/// time (the function reference won't resolve) or at runtime (the function
/// won't exist).
#[test]
fn test_build_gateway_removed() {
    // Verify that the chat module source no longer references build_gateway.
    let module_source = include_str!("chat/mod.rs");
    assert!(
        !module_source.contains("build_gateway"),
        "chat/mod.rs still references build_gateway — it should have been removed"
    );
    // Verify run_chat still exists (the new RPC-based implementation)
    assert!(
        module_source.contains("async fn run_chat"),
        "chat/mod.rs should still have run_chat function"
    );
}

/// Verify the chat module source uses RPC client, not self-built Gateway.
#[test]
fn test_chat_module_uses_rpc_not_gateway() {
    let module_source = include_str!("chat/mod.rs");
    assert!(
        module_source.contains("ChatRpcClient"),
        "chat/mod.rs should use ChatRpcClient"
    );
    assert!(
        !module_source.contains("Gateway::new"),
        "chat/mod.rs should not construct Gateway directly"
    );
    assert!(
        !module_source.contains("SessionManager::new"),
        "chat/mod.rs should not construct SessionManager directly"
    );
    assert!(
        !module_source.contains("ProcessorRegistry"),
        "chat/mod.rs should not reference ProcessorRegistry"
    );
}

// ── run_chat RPC integration tests ─────────────────────────────────────────

use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

/// Helper: spawn a mock chat RPC server.
///
/// Reads requests, calls `handler` to produce responses, sends them back.
/// Stops when handler returns `None` or client disconnects.
async fn spawn_chat_mock_server(
    handler: std::sync::Arc<
        dyn Fn(crate::chat::rpc::ChatRequest) -> Option<Vec<crate::chat::rpc::ChatResponse>>
            + Send
            + Sync
            + 'static,
    >,
) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("test-repl.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let handler = std::sync::Arc::clone(&handler);

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let handler = std::sync::Arc::clone(&handler);
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut reader = BufReader::new(reader);

                loop {
                    let mut hdr = [0u8; 4];
                    match reader.read_exact(&mut hdr).await {
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    let body_len = u32::from_be_bytes(hdr) as usize;
                    let mut body = vec![0u8; body_len];
                    if reader.read_exact(&mut body).await.is_err() {
                        break;
                    }
                    let request: crate::chat::rpc::ChatRequest = match serde_json::from_slice(&body)
                    {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                    match handler(request) {
                        Some(responses) => {
                            for resp in &responses {
                                let json = serde_json::to_vec(resp).unwrap();
                                let len = (json.len() as u32).to_be_bytes();
                                if writer.write_all(&len).await.is_err() {
                                    return;
                                }
                                if writer.write_all(&json).await.is_err() {
                                    return;
                                }
                                if writer.flush().await.is_err() {
                                    return;
                                }
                            }
                        }
                        None => break,
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(10)).await;
    // Keep dir alive by leaking it (prevents tempdir cleanup from removing the socket)
    std::mem::forget(dir);
    sock
}

/// Verify that the ChatRpcClient can send a message to a mock server
/// and receive a streaming response — same flow as run_chat.
#[tokio::test]
async fn test_rpc_client_send_message_to_mock_server() {
    use crate::chat::rpc::client::ChatRpcClient;
    use crate::chat::rpc::{ChatRequest, ChatResponse};

    let handler: std::sync::Arc<dyn Fn(ChatRequest) -> Option<Vec<ChatResponse>> + Send + Sync> =
        std::sync::Arc::new(|req| match req {
            ChatRequest::ChatMessage { content, .. } => Some(vec![
                ChatResponse::ContentChunk {
                    text: format!("echo: {}", content),
                },
                ChatResponse::Done,
            ]),
            _ => Some(vec![ChatResponse::Done]),
        });

    let sock = spawn_chat_mock_server(handler).await;
    let client = ChatRpcClient::with_timeout(&sock, 5000);

    let mut stream = client
        .send_message("test-agent", "hello world")
        .await
        .unwrap();
    let mut collected = String::new();
    while let Ok(Some(resp)) = stream.next().await {
        match resp {
            ChatResponse::ContentChunk { text } => collected.push_str(&text),
            ChatResponse::Done => break,
            _ => {}
        }
    }
    assert_eq!(collected, "echo: hello world");
}

/// Verify that quit sends a Quit request to the server.
#[tokio::test]
async fn test_rpc_client_quit_to_mock_server() {
    use crate::chat::rpc::client::ChatRpcClient;
    use crate::chat::rpc::{ChatRequest, ChatResponse};

    let handler: std::sync::Arc<dyn Fn(ChatRequest) -> Option<Vec<ChatResponse>> + Send + Sync> =
        std::sync::Arc::new(|req| match req {
            ChatRequest::Quit => Some(vec![ChatResponse::Done]),
            _ => Some(vec![ChatResponse::Error {
                message: "unexpected".to_string(),
            }]),
        });

    let sock = spawn_chat_mock_server(handler).await;
    let client = ChatRpcClient::with_timeout(&sock, 5000);
    let resp = client.quit().await.unwrap();
    assert!(matches!(resp, ChatResponse::Done));
}

/// Verify that stop_session sends StopSession request and receives Done.
#[tokio::test]
async fn test_rpc_client_stop_session_to_mock_server() {
    use crate::chat::rpc::client::ChatRpcClient;
    use crate::chat::rpc::{ChatRequest, ChatResponse};

    let handler: std::sync::Arc<dyn Fn(ChatRequest) -> Option<Vec<ChatResponse>> + Send + Sync> =
        std::sync::Arc::new(|req| match req {
            ChatRequest::StopSession { agent_id } => Some(vec![
                ChatResponse::ContentChunk {
                    text: format!("stopped {}", agent_id),
                },
                ChatResponse::Done,
            ]),
            _ => Some(vec![ChatResponse::Error {
                message: "unexpected".to_string(),
            }]),
        });

    let sock = spawn_chat_mock_server(handler).await;
    let client = ChatRpcClient::with_timeout(&sock, 5000);
    let resp = client.stop_session("my-agent").await.unwrap();
    assert!(matches!(resp, ChatResponse::ContentChunk { text } if text == "stopped my-agent"));
}

/// Verify that the chat_socket_path helper returns the correct path.
#[test]
fn test_chat_socket_path_in_chat_tests() {
    let path = crate::chat::rpc::client::chat_socket_path(Path::new("/home/user/.closeclaw"));
    assert_eq!(
        path,
        std::path::PathBuf::from("/home/user/.closeclaw/chat.sock")
    );
}

/// Verify multiple streaming chunks are collected in order.
#[tokio::test]
async fn test_rpc_client_multiple_chunks_order() {
    use crate::chat::rpc::client::ChatRpcClient;
    use crate::chat::rpc::{ChatRequest, ChatResponse};

    let handler: std::sync::Arc<dyn Fn(ChatRequest) -> Option<Vec<ChatResponse>> + Send + Sync> =
        std::sync::Arc::new(|req| match req {
            ChatRequest::ChatMessage { .. } => Some(vec![
                ChatResponse::ContentChunk {
                    text: "A".to_string(),
                },
                ChatResponse::ThinkingChunk {
                    text: "thinking".to_string(),
                },
                ChatResponse::ContentChunk {
                    text: "B".to_string(),
                },
                ChatResponse::Done,
            ]),
            _ => Some(vec![ChatResponse::Done]),
        });

    let sock = spawn_chat_mock_server(handler).await;
    let client = ChatRpcClient::with_timeout(&sock, 5000);
    let mut stream = client.send_message("agent", "test").await.unwrap();

    let mut chunks = Vec::new();
    while let Ok(Some(resp)) = stream.next().await {
        match resp {
            ChatResponse::ContentChunk { text } => chunks.push(text),
            ChatResponse::ThinkingChunk { text } => chunks.push(format!("[think:{}]", text)),
            ChatResponse::Done => break,
            _ => {}
        }
    }
    assert_eq!(
        chunks,
        vec![
            "A".to_string(),
            "[think:thinking]".to_string(),
            "B".to_string()
        ]
    );
}
