//! Chat module — in-process TCP server for interactive chat sessions
//!
//! The chat server listens on `127.0.0.1:18889` and speaks a simple
//! JSON newline-delimited protocol.
//!
//! ## Protocol
//!
//! Client → Server:
//! ```json
//! {"type": "chat.start", "agent_id": "guide", "id": "uuid"}
//! {"type": "chat.message", "content": "...", "id": "uuid"}
//! {"type": "chat.stop", "id": "uuid"}
//! ```
//!
//! Server → Client:
//! ```json
//! {"type": "chat.started", "session_id": "uuid", "id": "uuid"}
//! {"type": "chat.response", "content": "...", "done": false, "id": "uuid"}
//! {"type": "chat.response.done", "id": "uuid"}
//! {"type": "chat.error", "message": "...", "id": "uuid"}
//! ```

pub mod protocol;
pub mod server;
pub mod session;

pub use protocol::{ClientMessage, ServerMessage};
pub use server::{spawn_chat_server, ChatServer};

#[cfg(test)]
mod tests {
    use super::protocol::*;

    #[test]
    fn test_client_message_variants() {
        // chat.start
        let json = r#"{"type":"chat.start","agent_id":"guide","id":"123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::ChatStart { .. }));

        // chat.message
        let json = r#"{"type":"chat.message","content":"hello world","id":"456"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::ChatMessage { .. }));

        // chat.stop
        let json = r#"{"type":"chat.stop","id":"789"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::ChatStop { .. }));
    }

    #[test]
    fn test_server_message_variants() {
        // chat.started
        let msg = ServerMessage::ChatStarted {
            session_id: "sess-1".to_string(),
            id: "req-1".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("chat.started"));

        // chat.response
        let msg = ServerMessage::ChatResponse {
            content: "hello".to_string(),
            done: true,
            id: "req-2".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("chat.response"));
        assert!(json.contains(r#""done":true"#));

        // chat.response.done
        let msg = ServerMessage::ChatResponseDone { id: "req-3".to_string() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("chat.response.done"));

        // chat.error
        let msg = ServerMessage::ChatError {
            message: "oops".to_string(),
            id: "req-4".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("chat.error"));
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let result: Result<ClientMessage, _> = serde_json::from_str("not json");
        assert!(result.is_err());

        let result: Result<ClientMessage, _> = serde_json::from_str(r#"{"type":"unknown"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_id_roundtrip() {
        let msg = ServerMessage::ChatStarted {
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            id: "abc".to_string(),
        };
        let json = msg.to_json().unwrap();
        let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            ServerMessage::ChatStarted { session_id, .. } => {
                assert_eq!(session_id, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("expected ChatStarted"),
        }
    }

    #[test]
    fn test_empty_content_handled() {
        let json = r#"{"type":"chat.message","content":"","id":"empty-1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::ChatMessage { content, .. } => {
                assert_eq!(content, "");
            }
            _ => panic!("expected ChatMessage"),
        }
    }
}
