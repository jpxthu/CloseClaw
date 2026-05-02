use super::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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

#[test]
fn resolve_agent_id_uses_cli_param() {
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".into(),
        agent_id: "custom".into(),
    };
    assert_eq!(cmd.resolve_agent_id(), "custom");
}

#[test]
fn resolve_agent_id_uses_env_var() {
    std::env::set_var("CLOSEWCLAW_DEFAULT_AGENT", "env-agent");
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".into(),
        agent_id: "guide".into(),
    };
    let r = cmd.resolve_agent_id();
    std::env::remove_var("CLOSEWCLAW_DEFAULT_AGENT");
    assert_eq!(r, "env-agent");
}

#[test]
fn resolve_agent_id_falls_back_to_guide() {
    std::env::remove_var("CLOSEWCLAW_DEFAULT_AGENT");
    let cmd = ChatCommand {
        message: None,
        addr: "127.0.0.1:18889".into(),
        agent_id: "guide".into(),
    };
    assert_eq!(cmd.resolve_agent_id(), "guide");
}

#[tokio::test]
async fn handle_server_message_variants() {
    let cases: Vec<(Option<&str>, bool)> = vec![
        (
            Some(r#"{"type":"chat.response","content":"hi","id":"1"}"#),
            false,
        ),
        (Some(r#"{"type":"chat.response.done","id":"1"}"#), false),
        (
            Some(r#"{"type":"chat.error","message":"boom","id":"1"}"#),
            false,
        ),
        (Some(r#"{"type":"unknown"}"#), false),
        (Some("not json"), false),
        (None, true),
    ];
    for (input, expected) in cases {
        let result = ChatCommand::handle_server_message(input.map(String::from))
            .await
            .unwrap();
        assert_eq!(result, expected, "failed for {:?}", input);
    }
}

async fn bind_accept() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let _ = l.accept().await;
    });
    (addr, h)
}

#[tokio::test]
async fn handle_stdin_line_quit_and_exit() {
    for input in &["quit", "exit"] {
        let (addr, h) = bind_accept().await;
        let s = TcpStream::connect(addr).await.unwrap();
        let (_, mut w) = tokio::io::split(s);
        assert_eq!(
            ChatCommand::handle_stdin_line(Some(input.to_string()), &mut w)
                .await
                .unwrap(),
            true
        );
        h.abort();
    }
}

#[tokio::test]
async fn handle_stdin_line_empty_and_whitespace() {
    for input in &["", "   "] {
        let (addr, h) = bind_accept().await;
        let s = TcpStream::connect(addr).await.unwrap();
        let (_, mut w) = tokio::io::split(s);
        assert_eq!(
            ChatCommand::handle_stdin_line(Some(input.to_string()), &mut w)
                .await
                .unwrap(),
            false
        );
        h.abort();
    }
}

#[tokio::test]
async fn handle_stdin_line_normal_message() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let h = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).await.unwrap();
        tx.send(String::from_utf8(buf[..n].to_vec()).unwrap())
            .unwrap();
    });
    let s = TcpStream::connect(addr).await.unwrap();
    let (_, mut w) = tokio::io::split(s);
    assert_eq!(
        ChatCommand::handle_stdin_line(Some("hello".into()), &mut w)
            .await
            .unwrap(),
        false
    );
    let v: serde_json::Value = serde_json::from_str(rx.await.unwrap().trim()).unwrap();
    assert_eq!(v["type"], "chat.message");
    assert_eq!(v["content"], "hello");
    h.abort();
}

#[tokio::test]
async fn handle_stdin_line_none_eof() {
    let (addr, h) = bind_accept().await;
    let s = TcpStream::connect(addr).await.unwrap();
    let (_, mut w) = tokio::io::split(s);
    assert_eq!(
        ChatCommand::handle_stdin_line(None, &mut w).await.unwrap(),
        true
    );
    h.abort();
}

async fn mock_server_seq(
    responses: Vec<serde_json::Value>,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let _ = ready_tx.send(());
        if let Ok((mut s, _)) = l.accept().await {
            for (i, resp) in responses.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                let _ = s.write_all((resp.to_string() + "\n").as_bytes()).await;
                let _ = s.flush().await;
            }
        }
    });
    (addr, h, ready_rx)
}

#[tokio::test]
async fn start_session_success() {
    let (addr, h, ready_rx) = mock_server_seq(vec![
        json!({"type":"chat.started","session_id":"s1","id":"r1"}),
    ])
    .await;
    ready_rx.await.unwrap();
    let (stream, sid) = ChatCommand::start_session(addr, "agent", None)
        .await
        .unwrap();
    assert_eq!(sid, "s1");
    drop(stream);
    h.await.unwrap();
}

#[tokio::test]
async fn start_session_with_timeout_success() {
    // timeout = Some but server responds normally — no timeout fired.
    let (addr, h, ready_rx) = mock_server_seq(vec![
        json!({"type":"chat.started","session_id":"s1","id":"r1"}),
    ])
    .await;
    ready_rx.await.unwrap();
    let (stream, sid) = ChatCommand::start_session(addr, "agent", Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(sid, "s1");
    drop(stream);
    h.await.unwrap();
}

#[tokio::test]
async fn test_error_response_handling() {
    let (addr, h, ready_rx) = mock_server_seq(vec![
        json!({"type":"chat.error","message":"boom","id":"r1"}),
    ])
    .await;
    ready_rx.await.unwrap();
    assert!(ChatCommand::start_session(addr, "agent", None)
        .await
        .is_err());
    h.await.unwrap();
}

// NOTE: Mock server accepts the connection but never sends chat.started.
// start_session waits for the first line and should hit read_line_timeout.
#[tokio::test]
async fn test_read_timeout_silent_server() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let h = tokio::spawn(async move {
        let _ = ready_tx.send(());
        if let Ok((mut s, _)) = l.accept().await {
            // Read the chat.start request but never respond.
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            // Intentionally never write — simulates silent server.
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });
    ready_rx.await.unwrap();
    let result = ChatCommand::start_session(addr, "agent", Some(Duration::from_secs(1))).await;
    assert!(result.is_err(), "expected timeout error, got {:?}", result);
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("read timeout"),
        "error should mention 'read timeout', got: {}",
        err
    );
    h.abort();
}

#[tokio::test]
async fn test_connect_timeout() {
    // Non-routable address causes connect to time out (no ICMP, TCP SYN retransmits).
    let addr: std::net::SocketAddr = "10.255.255.1:1".parse().unwrap();
    let result = ChatCommand::start_session(addr, "agent", Some(Duration::from_secs(1))).await;
    assert!(
        result.is_err(),
        "expected connect timeout error, got {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("connect timeout"),
        "error should mention 'connect timeout', got: {}",
        err
    );
}

#[tokio::test]
async fn send_user_message_sends_correct_json() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(v["type"], "chat.message");
        assert_eq!(v["content"], "hello");
    });
    let mut s = TcpStream::connect(addr).await.unwrap();
    ChatCommand::send_user_message(&mut s, "hello")
        .await
        .unwrap();
    h.await.unwrap();
}

#[tokio::test]
async fn send_stop_sends_correct_json() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(v["type"], "chat.stop");
    });
    let mut s = TcpStream::connect(addr).await.unwrap();
    ChatCommand::send_stop(&mut s).await.unwrap();
    h.await.unwrap();
}

#[tokio::test]
async fn handle_single_response_normal() {
    let (addr, h, ready_rx) = mock_server_seq(vec![
        json!({"type":"chat.response","content":"ok","id":"r1"}),
        json!({"type":"chat.response.done","id":"r1"}),
    ])
    .await;
    ready_rx.await.unwrap();
    let mut s = TcpStream::connect(addr).await.unwrap();
    assert!(ChatCommand::handle_single_response(&mut s).await.is_ok());
    h.await.unwrap();
}

#[tokio::test]
async fn handle_single_response_error() {
    let (addr, h, ready_rx) = mock_server_seq(vec![
        json!({"type":"chat.response","content":"ok","id":"r1"}),
        json!({"type":"chat.error","message":"boom","id":"r1"}),
    ])
    .await;
    ready_rx.await.unwrap();
    let mut s = TcpStream::connect(addr).await.unwrap();
    assert!(ChatCommand::handle_single_response(&mut s).await.is_err());
    h.await.unwrap();
}

#[tokio::test]
async fn send_json_line_produces_newline_delimited_json() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).await.unwrap();
        assert!(buf[..n].ends_with(&[b'\n']));
        let nl = buf[..n].iter().position(|&b| b == b'\n').unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf[..nl]).unwrap();
        assert_eq!(v["type"], "test");
    });
    let mut s = TcpStream::connect(addr).await.unwrap();
    send_json_line(&mut s, &json!({"type":"test"}))
        .await
        .unwrap();
    h.await.unwrap();
}

#[tokio::test]
#[ignore = "CI environment memory不足时跳过"]
async fn run_single_end_to_end() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).await.unwrap();
        let sv: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(sv["type"], "chat.start");
        let started = json!({"type":"chat.started","session_id":"t","id":sv["id"]});
        s.write_all((started.to_string() + "\n").as_bytes())
            .await
            .unwrap();
        s.flush().await.unwrap();
        let n = s.read(&mut buf).await.unwrap();
        let mv: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(mv["type"], "chat.message");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let resp = json!({"type":"chat.response","content":"ok","id":mv["id"]});
        s.write_all((resp.to_string() + "\n").as_bytes())
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let done = json!({"type":"chat.response.done","id":mv["id"]});
        s.write_all((done.to_string() + "\n").as_bytes())
            .await
            .unwrap();
        s.flush().await.unwrap();
        let n = s.read(&mut buf).await.unwrap();
        let stv: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(stv["type"], "chat.stop");
    });
    let cmd = ChatCommand {
        message: Some("test".into()),
        addr: addr.to_string(),
        agent_id: "test-agent".into(),
    };
    assert!(cmd.run_single(addr, "test-agent", "test").await.is_ok());
    h.await.unwrap();
}
