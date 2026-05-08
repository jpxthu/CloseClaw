//! Helpers for multi-session chat tests

#[cfg(test)]
mod multi_session_tests {
    use super::*;
    use crate::chat::protocol::ServerMessage;
    use crate::chat::session::ChatSession;
    use crate::llm::stub::StubProvider;
    use crate::llm::LLMRegistry;
    use std::sync::Arc;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::broadcast,
    };

    async fn read_server_message(
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> ServerMessage {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("failed to parse: {}", e))
    }

    async fn send_client_json(writer: &mut tokio::net::tcp::OwnedWriteHalf, json: &str) {
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
    }

    async fn run_client_through_message(
        client: tokio::net::TcpStream,
        session_id: &str,
        req_id: &str,
    ) -> (
        String,
        BufReader<tokio::net::tcp::OwnedReadHalf>,
        tokio::net::tcp::OwnedWriteHalf,
    ) {
        let (reader_half, mut writer_half) = client.into_split();
        let mut reader = BufReader::new(reader_half);

        send_client_json(
            &mut writer_half,
            &format!(
                r#"{{"type":"chat.start","agent_id":"agent-{}","id":"{}-start"}}"#,
                session_id, req_id
            ),
        )
        .await;
        let start_msg = read_server_message(&mut reader).await;
        let received_sid = match start_msg {
            ServerMessage::ChatStarted {
                session_id: sid,
                id,
            } => {
                assert_eq!(id, format!("{}-start", req_id));
                sid
            }
            other => panic!("expected ChatStarted, got {:?}", other),
        };
        assert_eq!(received_sid, session_id);

        send_client_json(
            &mut writer_half,
            &format!(
                r#"{{"type":"chat.message","content":"hello from {}","id":"{}-msg"}}"#,
                session_id, req_id
            ),
        )
        .await;
        let msg_a = read_server_message(&mut reader).await;
        let msg_b = read_server_message(&mut reader).await;
        match (&msg_a, &msg_b) {
            (
                ServerMessage::ChatResponse { content, done, id },
                ServerMessage::ChatResponseDone { id: id2 },
            ) => {
                assert_eq!(*content, "stub response");
                assert!(*done);
                assert_eq!(*id, format!("{}-msg", req_id));
                assert_eq!(*id2, format!("{}-msg", req_id));
            }
            _ => panic!("expected ChatResponse + ChatResponseDone"),
        }
        (received_sid, reader, writer_half)
    }

    async fn expect_shutdown_error(
        mut reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
        label: &str,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let msg: ServerMessage = serde_json::from_str(line.trim()).unwrap();
        match msg {
            ServerMessage::ChatError { message, .. } => {
                assert!(
                    message.contains("server shutting down"),
                    "expected 'server shutting down' for {}, got: {}",
                    label,
                    message
                );
            }
            other => panic!("expected ChatError for {}, got {:?}", label, other),
        }
    }

    #[tokio::test]
    async fn test_multi_client_concurrent_sessions() {
        std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");

        let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);
        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("stub".to_string(), Arc::new(StubProvider::new()))
            .await;

        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port1 = listener1.local_addr().unwrap().port();
        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port2 = listener2.local_addr().unwrap().port();
        let listener3 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port3 = listener3.local_addr().unwrap().port();

        let reg1 = Arc::clone(&registry);
        let rx1 = shutdown_rx.resubscribe();
        let accept1 = tokio::spawn(async move {
            let (accepted, _) = listener1.accept().await.unwrap();
            ChatSession::new(
                "session-1".to_string(),
                "test-agent".to_string(),
                accepted,
                rx1,
                reg1,
            )
        });

        let reg2 = Arc::clone(&registry);
        let rx2 = shutdown_rx.resubscribe();
        let accept2 = tokio::spawn(async move {
            let (accepted, _) = listener2.accept().await.unwrap();
            ChatSession::new(
                "session-2".to_string(),
                "test-agent".to_string(),
                accepted,
                rx2,
                reg2,
            )
        });

        let reg3 = Arc::clone(&registry);
        let rx3 = shutdown_rx.resubscribe();
        let accept3 = tokio::spawn(async move {
            let (accepted, _) = listener3.accept().await.unwrap();
            ChatSession::new(
                "session-3".to_string(),
                "test-agent".to_string(),
                accepted,
                rx3,
                reg3,
            )
        });

        let client1 = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port1))
            .await
            .unwrap();
        let client2 = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port2))
            .await
            .unwrap();
        let client3 = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port3))
            .await
            .unwrap();

        let session1 = accept1.await.unwrap();
        let session2 = accept2.await.unwrap();
        let session3 = accept3.await.unwrap();

        let handle1 = tokio::spawn(session1.run());
        let handle2 = tokio::spawn(session2.run());
        let handle3 = tokio::spawn(session3.run());

        let (sid1, reader1, writer1) = run_client_through_message(client1, "session-1", "c1").await;
        let (sid2, reader2, writer2) = run_client_through_message(client2, "session-2", "c2").await;
        let (sid3, reader3, writer3) = run_client_through_message(client3, "session-3", "c3").await;

        assert_ne!(sid1, sid2, "session IDs distinct");
        assert_ne!(sid1, sid3, "session IDs distinct");
        assert_ne!(sid2, sid3, "session IDs distinct");

        drop(shutdown_tx);

        let (r1, r2, r3) = tokio::join!(
            expect_shutdown_error(reader1, "client1"),
            expect_shutdown_error(reader2, "client2"),
            expect_shutdown_error(reader3, "client3"),
        );
        let _ = (r1, r2, r3);

        std::env::remove_var("LLM_FALLBACK_CHAIN");
        let _ = (writer1, writer2, writer3);
        let _ = handle1.await;
        let _ = handle2.await;
        let _ = handle3.await;
    }

    #[tokio::test]
    async fn test_multi_client_history_isolation() {
        std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");

        let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);
        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("stub".to_string(), Arc::new(StubProvider::new()))
            .await;

        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_a = listener_a.local_addr().unwrap().port();
        let reg_a = Arc::clone(&registry);
        let rx_a = shutdown_rx.resubscribe();
        let accept_a = tokio::spawn(async move {
            let (accepted, _) = listener_a.accept().await.unwrap();
            ChatSession::new(
                "session-a".to_string(),
                "test-agent".to_string(),
                accepted,
                rx_a,
                reg_a,
            )
        });
        let client_a = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port_a))
            .await
            .unwrap();
        let session_a = accept_a.await.unwrap();
        let handle_a = tokio::spawn(session_a.run());

        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_b = listener_b.local_addr().unwrap().port();
        let reg_b = Arc::clone(&registry);
        let rx_b = shutdown_rx.resubscribe();
        let accept_b = tokio::spawn(async move {
            let (accepted, _) = listener_b.accept().await.unwrap();
            ChatSession::new(
                "session-b".to_string(),
                "test-agent".to_string(),
                accepted,
                rx_b,
                reg_b,
            )
        });
        let client_b = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port_b))
            .await
            .unwrap();
        let session_b = accept_b.await.unwrap();
        let handle_b = tokio::spawn(session_b.run());

        std::env::remove_var("LLM_FALLBACK_CHAIN");

        let (reader_a, mut writer_a) = client_a.into_split();
        let mut reader_a = BufReader::new(reader_a);
        let (reader_b, mut writer_b) = client_b.into_split();
        let mut reader_b = BufReader::new(reader_b);

        // Round 1: start
        send_client_json(
            &mut writer_a,
            r#"{"type":"chat.start","agent_id":"agent-a","id":"a-start-1"}"#,
        )
        .await;
        send_client_json(
            &mut writer_b,
            r#"{"type":"chat.start","agent_id":"agent-b","id":"b-start-1"}"#,
        )
        .await;

        let start_a = read_server_message(&mut reader_a).await;
        let start_b = read_server_message(&mut reader_b).await;
        match (&start_a, &start_b) {
            (
                ServerMessage::ChatStarted {
                    session_id: sid_a,
                    id: id_a,
                },
                ServerMessage::ChatStarted {
                    session_id: sid_b,
                    id: id_b,
                },
            ) => {
                assert_eq!(sid_a, "session-a");
                assert_eq!(id_a, "a-start-1");
                assert_eq!(sid_b, "session-b");
                assert_eq!(id_b, "b-start-1");
            }
            _ => panic!("expected ChatStarted"),
        }

        // Round 1: message
        send_client_json(
            &mut writer_a,
            r#"{"type":"chat.message","content":"hello from A","id":"a-msg-1"}"#,
        )
        .await;
        send_client_json(
            &mut writer_b,
            r#"{"type":"chat.message","content":"hello from B","id":"b-msg-1"}"#,
        )
        .await;

        let resp_a1 = read_server_message(&mut reader_a).await;
        let done_a1 = read_server_message(&mut reader_a).await;
        match (&resp_a1, &done_a1) {
            (
                ServerMessage::ChatResponse { id, .. },
                ServerMessage::ChatResponseDone { id: id2 },
            ) => {
                assert_eq!(*id, "a-msg-1");
                assert_eq!(*id2, "a-msg-1");
            }
            _ => panic!("expected response"),
        }
        let resp_b1 = read_server_message(&mut reader_b).await;
        let done_b1 = read_server_message(&mut reader_b).await;
        match (&resp_b1, &done_b1) {
            (
                ServerMessage::ChatResponse { id, .. },
                ServerMessage::ChatResponseDone { id: id2 },
            ) => {
                assert_eq!(*id, "b-msg-1");
                assert_eq!(*id2, "b-msg-1");
            }
            _ => panic!("expected response"),
        }

        // Round 2
        send_client_json(
            &mut writer_a,
            r#"{"type":"chat.message","content":"hello again from A","id":"a-msg-2"}"#,
        )
        .await;
        send_client_json(
            &mut writer_b,
            r#"{"type":"chat.message","content":"hello again from B","id":"b-msg-2"}"#,
        )
        .await;

        let resp_a2 = read_server_message(&mut reader_a).await;
        let done_a2 = read_server_message(&mut reader_a).await;
        match (&resp_a2, &done_a2) {
            (
                ServerMessage::ChatResponse { id, .. },
                ServerMessage::ChatResponseDone { id: id2 },
            ) => {
                assert_eq!(*id, "a-msg-2");
                assert_eq!(*id2, "a-msg-2");
            }
            _ => panic!("expected response"),
        }
        let resp_b2 = read_server_message(&mut reader_b).await;
        let done_b2 = read_server_message(&mut reader_b).await;
        match (&resp_b2, &done_b2) {
            (
                ServerMessage::ChatResponse { id, .. },
                ServerMessage::ChatResponseDone { id: id2 },
            ) => {
                assert_eq!(*id, "b-msg-2");
                assert_eq!(*id2, "b-msg-2");
            }
            _ => panic!("expected response"),
        }

        drop(shutdown_tx);

        let (err_a, err_b) = tokio::join!(
            expect_shutdown_error(reader_a, "client-a"),
            expect_shutdown_error(reader_b, "client-b"),
        );
        let _ = (err_a, err_b);

        let _ = writer_a;
        let _ = writer_b;
        let _ = handle_a.await;
        let _ = handle_b.await;
    }
}
