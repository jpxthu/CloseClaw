use super::*;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

// -------------------------------------------------------------------------
// ChatCommand default-parameter tests
// -------------------------------------------------------------------------

#[test]
fn chat_command_defaults() {
    let cmd = ChatCommand::parse_from(&["chat"]);
    assert_eq!(cmd.agent_id, "guide");
    assert_eq!(cmd.addr, "127.0.0.1:18889");
    assert!(cmd.message.is_none());
}

#[test]
fn chat_command_with_message() {
    let cmd = ChatCommand::parse_from(&["chat", "--message", "hello"]);
    assert_eq!(cmd.message, Some("hello".to_string()));
}

// -------------------------------------------------------------------------
// resolve_agent_id tests
// -------------------------------------------------------------------------

#[test]
fn resolve_agent_id_uses_cli_param_when_not_default() {
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".to_string(),
        agent_id: "custom-agent".to_string(),
    };
    assert_eq!(cmd.resolve_agent_id(), "custom-agent");
}

#[test]
fn resolve_agent_id_uses_env_var_when_default() {
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".to_string(),
        agent_id: "guide".to_string(), // default
    };
    std::env::set_var("CLOSEWCLAW_DEFAULT_AGENT", "env-agent");
    let result = cmd.resolve_agent_id();
    std::env::remove_var("CLOSEWCLAW_DEFAULT_AGENT");
    assert_eq!(result, "env-agent");
}

#[test]
fn resolve_agent_id_falls_back_to_guide_when_env_not_set() {
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".to_string(),
        agent_id: "guide".to_string(),
    };
    std::env::remove_var("CLOSEWCLAW_DEFAULT_AGENT");
    assert_eq!(cmd.resolve_agent_id(), "guide");
}

// -------------------------------------------------------------------------
// handle_server_message tests
// -------------------------------------------------------------------------

#[test]
fn handle_server_message_chat_response() {
    let val = json!({"type": "chat.response", "content": "hello", "id": "123"});
    let result = ChatCommand::handle_server_message(&val);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false); // not done
}

#[test]
fn handle_server_message_chat_response_done() {
    let val = json!({"type": "chat.response.done", "id": "123"});
    let result = ChatCommand::handle_server_message(&val);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), true); // done
}

#[test]
fn handle_server_message_chat_error() {
    let val = json!({"type": "chat.error", "message": "boom", "id": "123"});
    let result = ChatCommand::handle_server_message(&val);
    assert!(result.is_err());
}

#[test]
fn handle_server_message_unknown_type() {
    let val = json!({"type": "unknown", "foo": "bar"});
    let result = ChatCommand::handle_server_message(&val);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false);
}

#[test]
fn handle_server_message_invalid_json() {
    // JSON with extra spaces/newlines should be printed raw
    let val = serde_json::Value::String("  { invalid }  ".to_string());
    let result = ChatCommand::handle_server_message(&val);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), false);
}

// -------------------------------------------------------------------------
// handle_stdin_line tests
// -------------------------------------------------------------------------

#[test]
fn handle_stdin_line_quit() {
    let result = ChatCommand::handle_stdin_line(Some("quit".to_string()));
    assert_eq!(result.unwrap(), false); // should stop
}

#[test]
fn handle_stdin_line_exit() {
    let result = ChatCommand::handle_stdin_line(Some("exit".to_string()));
    assert_eq!(result.unwrap(), false); // should stop
}

#[test]
fn handle_stdin_line_empty() {
    let result = ChatCommand::handle_stdin_line(Some("".to_string()));
    assert_eq!(result.unwrap(), true); // should continue
}

#[test]
fn handle_stdin_line_normal_message() {
    let result = ChatCommand::handle_stdin_line(Some("hello world".to_string()));
    assert_eq!(result.unwrap(), true); // should continue
}

#[test]
fn handle_stdin_line_none_eof() {
    let result = ChatCommand::handle_stdin_line(None);
    assert_eq!(result.unwrap(), false); // should stop on EOF
}

// -------------------------------------------------------------------------
// TCP mock helpers
// -------------------------------------------------------------------------

/// Start a mock TCP server that sends a sequence of responses.
/// Returns the listener address and a handle to the spawned task.
async fn mock_server_seq(responses: Vec<serde_json::Value>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::AsyncWriteExt;
            for resp in responses {
                let line = resp.to_string() + "\n";
                let _ = stream.write_all(line.as_bytes()).await;
                let _ = stream.flush().await;
            }
        }
    });
    (addr, handle)
}

// -------------------------------------------------------------------------
// start_session tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn start_session_success() {
    let (addr, handle) = mock_server_seq(vec![
        json!({"type": "chat.started", "session_id": "test-session", "id": "req-123"}),
    ]).await;
    
    let result = ChatCommand::start_session(addr, "test-agent").await;
    assert!(result.is_ok());
    let (_stream, session_id) = result.unwrap();
    assert_eq!(session_id, "test-session");
    
    handle.await.unwrap();
}

#[tokio::test]
async fn start_session_fails_on_wrong_response_type() {
    let (addr, handle) = mock_server_seq(vec![
        json!({"type": "chat.error", "message": "boom", "id": "req-123"}),
    ]).await;
    
    let result = ChatCommand::start_session(addr, "test-agent").await;
    assert!(result.is_err());
    
    handle.await.unwrap();
}

// -------------------------------------------------------------------------
// send_user_message tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn send_user_message_sends_correct_json() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    let server_handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let data = &buf[..n];
            assert!(data.ends_with(&[b'\n']));
            let line = std::str::from_utf8(&data[..data.len()-1]).unwrap();
            let val: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(val.get("type").and_then(|v| v.as_str()), Some("chat.message"));
            assert_eq!(val.get("content").and_then(|v| v.as_str()), Some("hello"));
            assert!(val.get("id").is_some());
        }
    });
    
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let result = ChatCommand::send_user_message(&mut stream, "hello").await;
    assert!(result.is_ok());
    
    server_handle.await.unwrap();
}

// -------------------------------------------------------------------------
// send_stop tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn send_stop_sends_correct_json() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    let server_handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let data = &buf[..n];
            assert!(data.ends_with(&[b'\n']));
            let line = std::str::from_utf8(&data[..data.len()-1]).unwrap();
            let val: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(val.get("type").and_then(|v| v.as_str()), Some("chat.stop"));
            assert!(val.get("id").is_some());
        }
    });
    
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let result = ChatCommand::send_stop(&mut stream).await;
    assert!(result.is_ok());
    
    server_handle.await.unwrap();
}

// -------------------------------------------------------------------------
// run_single (end‑to‑end) test
// -------------------------------------------------------------------------

#[tokio::test]
#[ignore = "CI environment memory不足时跳过"]
async fn run_single_end_to_end() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    let server_handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            // Read chat.start
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let start_line = std::str::from_utf8(&buf[..n]).unwrap();
            let start_val: serde_json::Value = serde_json::from_str(start_line.trim()).unwrap();
            assert_eq!(start_val.get("type").and_then(|v| v.as_str()), Some("chat.start"));
            
            // Send chat.started
            let started = json!({"type": "chat.started", "session_id": "test", "id": start_val["id"]});
            stream.write_all((started.to_string() + "\n").as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            
            // Read chat.message
            let n = stream.read(&mut buf).await.unwrap();
            let msg_line = std::str::from_utf8(&buf[..n]).unwrap();
            let msg_val: serde_json::Value = serde_json::from_str(msg_line.trim()).unwrap();
            assert_eq!(msg_val.get("type").and_then(|v| v.as_str()), Some("chat.message"));
            
            // Send chat.response + chat.response.done
            let resp = json!({"type": "chat.response", "content": "ok", "id": msg_val["id"]});
            stream.write_all((resp.to_string() + "\n").as_bytes()).await.unwrap();
            let done = json!({"type": "chat.response.done", "id": msg_val["id"]});
            stream.write_all((done.to_string() + "\n").as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            
            // Read chat.stop
            let n = stream.read(&mut buf).await.unwrap();
            let stop_line = std::str::from_utf8(&buf[..n]).unwrap();
            let stop_val: serde_json::Value = serde_json::from_str(stop_line.trim()).unwrap();
            assert_eq!(stop_val.get("type").and_then(|v| v.as_str()), Some("chat.stop"));
        }
    });
    
    let cmd = ChatCommand {
        message: Some("test".to_string()),
        addr: addr.to_string(),
        agent_id: "test-agent".to_string(),
    };
    
    let result = cmd.run_single(addr, "test-agent", "test").await;
    assert!(result.is_ok());
    
    server_handle.await.unwrap();
}

// -------------------------------------------------------------------------
// handle_single_response tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn handle_single_response_normal_sequence() {
    let (addr, handle) = mock_server_seq(vec![
        json!({"type": "chat.response", "content": "ok", "id": "req-123"}),
        json!({"type": "chat.response.done", "id": "req-123"}),
    ]).await;
    
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let result = ChatCommand::handle_single_response(&mut stream).await;
    assert!(result.is_ok());
    
    handle.await.unwrap();
}

#[tokio::test]
async fn handle_single_response_error() {
    let (addr, handle) = mock_server_seq(vec![
        json!({"type": "chat.response", "content": "ok", "id": "req-123"}),
        json!({"type": "chat.error", "message": "boom", "id": "req-123"}),
    ]).await;
    
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let result = ChatCommand::handle_single_response(&mut stream).await;
    assert!(result.is_err());
    
    handle.await.unwrap();
}

// -------------------------------------------------------------------------
// send_json_line tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn send_json_line_produces_newline_delimited_json() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    let server_handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let data = &buf[..n];
            assert!(data.ends_with(&[b'\n']));
            let line_end = data.iter().position(|&b| b == b'\n').unwrap();
            let val: serde_json::Value = serde_json::from_slice(&data[..line_end]).unwrap();
            assert_eq!(val.get("type").and_then(|v| v.as_str()), Some("test"));
        }
    });
    
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let val = json!({"type": "test"});
    send_json_line(&mut stream, &val).await.unwrap();
    
    server_handle.await.unwrap();
}