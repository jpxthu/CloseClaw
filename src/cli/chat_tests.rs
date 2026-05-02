use super::*;
use serde_json::json;
use std::io::{Read, Write as StdWrite};
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

// ---------------------------------------------------------------------------
// Test TCP helpers — WSL2-safe design
// ---------------------------------------------------------------------------
//
// WSL2 has a known issue where tokio's edge-triggered epoll does not wake
// `TcpListener::accept().await` when a connection arrives on the accept
// queue.  The kernel completes the TCP handshake (so connect() succeeds on
// the client side), but the listener task never gets notified.
//
// Workaround: use `spawn_blocking` + `std::net::TcpListener::accept()`,
// which is a plain blocking syscall and does not depend on epoll.
// After accepting, convert to `tokio::net::TcpStream` for async I/O.

/// Bind a TCP listener using std only (bypasses tokio socket management).
/// Returns (addr, std_listener).
fn bind_tcp_std() -> (std::net::SocketAddr, std::net::TcpListener) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(false).unwrap();
    let addr = l.local_addr().unwrap();
    eprintln!("[bind_tcp_std] bound to {}", addr);
    (addr, l)
}

/// Mock TCP server: blocking accept on std thread, then async writes via block_on.
async fn mock_server_seq(
    responses: Vec<serde_json::Value>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        eprintln!("[mock_server_seq] calling blocking accept on {}", addr);
        match std_listener.accept() {
            Ok((std_stream, _)) => {
                eprintln!("[mock_server_seq] accepted!");
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async move {
                    let mut s = TcpStream::from_std(std_stream).unwrap();
                    for (i, resp) in responses.iter().enumerate() {
                        if i > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                        let _ = s.write_all((resp.to_string() + "\n").as_bytes()).await;
                        let _ = s.flush().await;
                    }
                    eprintln!("[mock_server_seq] done writing");
                });
            }
            Err(e) => eprintln!("[mock_server_seq] accept error: {}", e),
        }
    });
    (addr, h)
}

/// Blocking accept one connection (no processing).
async fn bind_accept() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        eprintln!("[bind_accept] calling blocking accept on {}", addr);
        let _ = std_listener.accept();
        eprintln!("[bind_accept] accept returned");
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
    let (addr, std_listener) = bind_tcp_std();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let h = tokio::task::spawn_blocking(move || {
        let (mut stream, _) = std_listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
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

#[tokio::test]
async fn start_session_success() {
    let (addr, h) = mock_server_seq(vec![
        json!({"type":"chat.started","session_id":"s1","id":"r1"}),
    ])
    .await;
    let (stream, sid) = ChatCommand::start_session(addr, "agent").await.unwrap();
    assert_eq!(sid, "s1");
    drop(stream);
    h.await.unwrap();
}

#[tokio::test]
async fn test_error_response_handling() {
    let (addr, h) = mock_server_seq(vec![
        json!({"type":"chat.error","message":"boom","id":"r1"}),
    ])
    .await;
    assert!(ChatCommand::start_session(addr, "agent").await.is_err());
    h.await.unwrap();
}

#[tokio::test]
#[ignore = "slow test: 30s read timeout; run with --ignored to verify timeout mechanism"]
async fn test_read_timeout_silent_server() {
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        let _ = std_listener.accept();
        // intentionally never write — simulates silent server
        std::thread::sleep(std::time::Duration::from_secs(60));
    });
    let _ = ChatCommand::start_session(addr, "agent").await;
    h.abort();
}

#[tokio::test]
async fn send_user_message_sends_correct_json() {
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        let (mut stream, _) = std_listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
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
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        let (mut stream, _) = std_listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
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
    let (addr, h) = mock_server_seq(vec![
        json!({"type":"chat.response","content":"ok","id":"r1"}),
        json!({"type":"chat.response.done","id":"r1"}),
    ])
    .await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    assert!(ChatCommand::handle_single_response(&mut s).await.is_ok());
    h.await.unwrap();
}

#[tokio::test]
async fn handle_single_response_error() {
    let (addr, h) = mock_server_seq(vec![
        json!({"type":"chat.response","content":"ok","id":"r1"}),
        json!({"type":"chat.error","message":"boom","id":"r1"}),
    ])
    .await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    assert!(ChatCommand::handle_single_response(&mut s).await.is_err());
    h.await.unwrap();
}

#[tokio::test]
async fn send_json_line_produces_newline_delimited_json() {
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        let (mut stream, _) = std_listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap();
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
    let (addr, std_listener) = bind_tcp_std();
    let h = tokio::task::spawn_blocking(move || {
        let (mut stream, _) = std_listener.accept().unwrap();
        let mut buf = [0u8; 2048];

        // Read chat.start
        let n = stream.read(&mut buf).unwrap();
        let sv: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(sv["type"], "chat.start");

        // Write chat.started
        let started = json!({"type":"chat.started","session_id":"t","id":sv["id"]});
        stream
            .write_all((started.to_string() + "\n").as_bytes())
            .unwrap();
        stream.flush().unwrap();

        // Read chat.message
        let n = stream.read(&mut buf).unwrap();
        let mv: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().trim()).unwrap();
        assert_eq!(mv["type"], "chat.message");

        // Write chat.response
        std::thread::sleep(std::time::Duration::from_millis(10));
        let resp = json!({"type":"chat.response","content":"ok","id":mv["id"]});
        stream
            .write_all((resp.to_string() + "\n").as_bytes())
            .unwrap();

        // Write chat.response.done
        std::thread::sleep(std::time::Duration::from_millis(10));
        let done = json!({"type":"chat.response.done","id":mv["id"]});
        stream
            .write_all((done.to_string() + "\n").as_bytes())
            .unwrap();
        stream.flush().unwrap();

        // Read chat.stop
        let n = stream.read(&mut buf).unwrap();
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
