//! Chat protocol — JSON over TCP message types

use serde::{Deserialize, Serialize};

/// Client → Server messages
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum ClientMessage {
    /// Start a new chat session
    #[serde(rename = "chat.start")]
    ChatStart {
        agent_id: String,
        id: String,
    },
    /// Send a chat message
    #[serde(rename = "chat.message")]
    ChatMessage {
        content: String,
        id: String,
    },
    /// Stop the current chat session
    #[serde(rename = "chat.stop")]
    ChatStop {
        id: String,
    },
}

/// Server → Client messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Session started
    #[serde(rename = "chat.started")]
    ChatStarted {
        session_id: String,
        id: String,
    },
    /// A response chunk
    #[serde(rename = "chat.response")]
    ChatResponse {
        content: String,
        done: bool,
        id: String,
    },
    /// Final response marker
    #[serde(rename = "chat.response.done")]
    ChatResponseDone {
        id: String,
    },
    /// Error occurred
    #[serde(rename = "chat.error")]
    ChatError {
        message: String,
        id: String,
    },
}

impl ServerMessage {
    /// Serialize to JSON string
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_start_deserialize() {
        let json = r#"{"type":"chat.start","agent_id":"guide","id":"abc123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::ChatStart { agent_id, id } => {
                assert_eq!(agent_id, "guide");
                assert_eq!(id, "abc123");
            }
            _ => panic!("expected ChatStart"),
        }
    }

    #[test]
    fn test_chat_message_deserialize() {
        let json = r#"{"type":"chat.message","content":"hello","id":"msg1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::ChatMessage { content, id } => {
                assert_eq!(content, "hello");
                assert_eq!(id, "msg1");
            }
            _ => panic!("expected ChatMessage"),
        }
    }

    #[test]
    fn test_chat_stop_deserialize() {
        let json = r#"{"type":"chat.stop","id":"stop1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::ChatStop { id } => {
                assert_eq!(id, "stop1");
            }
            _ => panic!("expected ChatStop"),
        }
    }

    #[test]
    fn test_chat_started_serialize() {
        let msg = ServerMessage::ChatStarted {
            session_id: "sess-abc".to_string(),
            id: "req1".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"chat.started""#));
        assert!(json.contains(r#""session_id":"sess-abc""#));
    }

    #[test]
    fn test_chat_response_serialize() {
        let msg = ServerMessage::ChatResponse {
            content: "hi there".to_string(),
            done: false,
            id: "req2".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"chat.response""#));
        assert!(json.contains(r#""done":false"#));
    }

    #[test]
    fn test_chat_error_serialize() {
        let msg = ServerMessage::ChatError {
            message: "something went wrong".to_string(),
            id: "req3".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"chat.error""#));
        assert!(json.contains(r#""message":"something went wrong""#));
    }

    #[test]
    fn test_roundtrip() {
        let started = ServerMessage::ChatStarted {
            session_id: "sess-xyz".to_string(),
            id: "id-1".to_string(),
        };
        let json = serde_json::to_string(&started).unwrap();
        assert!(json.contains("sess-xyz"));
    }
}
