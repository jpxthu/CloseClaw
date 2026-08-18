//! Chat RPC protocol types.
//!
//! Defines the request and response enums for CLI-to-daemon
//! chat communication over a Unix domain socket.
//!
//! Uses length-prefixed JSON frames:
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use serde::{Deserialize, Serialize};

/// Request sent from the CLI client to the chat RPC server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatRequest {
    /// Send a user message to the agent.
    ChatMessage {
        /// Target agent identifier.
        agent_id: String,
        /// User input content.
        content: String,
    },
    /// Terminate the current running session (corresponds to /stop).
    StopSession {
        /// Agent whose session to stop.
        agent_id: String,
    },
    /// Exit the chat session.
    Quit,
    /// Lightweight health check — server responds with Pong.
    Ping,
}

/// Response sent from the chat RPC server back to the CLI client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatResponse {
    /// Streaming text content chunk.
    ContentChunk {
        /// Rendered text fragment.
        text: String,
    },
    /// Thinking content chunk.
    ThinkingChunk {
        /// Thinking text fragment.
        text: String,
    },
    /// Tool use information.
    ToolUseChunk {
        /// Tool name.
        name: String,
        /// Tool input (serialized).
        input: String,
    },
    /// Tool result information.
    ToolResultChunk {
        /// Tool name.
        name: String,
        /// Tool output.
        output: String,
    },
    /// Session creation confirmation.
    SessionStarted {
        /// Session key.
        session_key: String,
    },
    /// Error response.
    Error {
        /// Error message.
        message: String,
    },
    /// Current response stream is complete.
    Done,
    /// Health check response.
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ChatRequest serialization tests ─────────────────────────────────

    #[test]
    fn test_chat_message_request_roundtrip() {
        let req = ChatRequest::ChatMessage {
            agent_id: "my-agent".to_string(),
            content: "Hello, world!".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_chat_message_request_json_structure() {
        let req = ChatRequest::ChatMessage {
            agent_id: "a".to_string(),
            content: "b".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "chat_message");
        assert_eq!(parsed["agent_id"], "a");
        assert_eq!(parsed["content"], "b");
    }

    #[test]
    fn test_stop_session_request_roundtrip() {
        let req = ChatRequest::StopSession {
            agent_id: "target-agent".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_stop_session_request_json_structure() {
        let req = ChatRequest::StopSession {
            agent_id: "x".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "stop_session");
        assert_eq!(parsed["agent_id"], "x");
    }

    #[test]
    fn test_quit_request_roundtrip() {
        let req = ChatRequest::Quit;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_quit_request_json_structure() {
        let req = ChatRequest::Quit;
        let json = serde_json::to_string(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "quit");
    }

    // ── Ping request tests ──────────────────────────────────────────────

    #[test]
    fn test_ping_request_roundtrip() {
        let req = ChatRequest::Ping;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        assert!(matches!(deserialized, ChatRequest::Ping));
    }

    #[test]
    fn test_ping_request_json_structure() {
        let req = ChatRequest::Ping;
        let json = serde_json::to_string(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "ping");
    }

    #[test]
    fn test_chat_message_empty_content() {
        let req = ChatRequest::ChatMessage {
            agent_id: "agent".to_string(),
            content: String::new(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        if let ChatRequest::ChatMessage { content, .. } = deserialized {
            assert!(content.is_empty());
        } else {
            panic!("expected ChatMessage variant");
        }
    }

    #[test]
    fn test_chat_message_unicode_content() {
        let req = ChatRequest::ChatMessage {
            agent_id: "agent".to_string(),
            content: "你好世界 🌍".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        if let ChatRequest::ChatMessage { content, .. } = deserialized {
            assert_eq!(content, "你好世界 🌍");
        } else {
            panic!("expected ChatMessage variant");
        }
    }

    // ── ChatResponse serialization tests ────────────────────────────────

    #[test]
    fn test_content_chunk_response_roundtrip() {
        let resp = ChatResponse::ContentChunk {
            text: "Hello!".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_content_chunk_json_structure() {
        let resp = ChatResponse::ContentChunk {
            text: "hi".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "content_chunk");
        assert_eq!(parsed["text"], "hi");
    }

    #[test]
    fn test_thinking_chunk_response_roundtrip() {
        let resp = ChatResponse::ThinkingChunk {
            text: "reasoning...".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_thinking_chunk_json_structure() {
        let resp = ChatResponse::ThinkingChunk {
            text: "thinking".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "thinking_chunk");
        assert_eq!(parsed["text"], "thinking");
    }

    #[test]
    fn test_tool_use_chunk_response_roundtrip() {
        let resp = ChatResponse::ToolUseChunk {
            name: "web_search".to_string(),
            input: r#"{"query":"rust async"}"#.to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_tool_use_chunk_json_structure() {
        let resp = ChatResponse::ToolUseChunk {
            name: "read".to_string(),
            input: "{}".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "tool_use_chunk");
        assert_eq!(parsed["name"], "read");
        assert_eq!(parsed["input"], "{}");
    }

    #[test]
    fn test_tool_result_chunk_response_roundtrip() {
        let resp = ChatResponse::ToolResultChunk {
            name: "web_search".to_string(),
            output: "found 3 results".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_tool_result_chunk_json_structure() {
        let resp = ChatResponse::ToolResultChunk {
            name: "exec".to_string(),
            output: "ok".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "tool_result_chunk");
        assert_eq!(parsed["name"], "exec");
        assert_eq!(parsed["output"], "ok");
    }

    #[test]
    fn test_session_started_response_roundtrip() {
        let resp = ChatResponse::SessionStarted {
            session_key: "terminal:u1:cli:owner".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_session_started_json_structure() {
        let resp = ChatResponse::SessionStarted {
            session_key: "key-123".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "session_started");
        assert_eq!(parsed["session_key"], "key-123");
    }

    #[test]
    fn test_error_response_roundtrip() {
        let resp = ChatResponse::Error {
            message: "something went wrong".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_error_response_json_structure() {
        let resp = ChatResponse::Error {
            message: "fail".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "fail");
    }

    #[test]
    fn test_done_response_roundtrip() {
        let resp = ChatResponse::Done;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_done_response_json_structure() {
        let resp = ChatResponse::Done;
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "done");
    }

    // ── Pong response tests ─────────────────────────────────────────────

    #[test]
    fn test_pong_response_roundtrip() {
        let resp = ChatResponse::Pong;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(deserialized, ChatResponse::Pong);
    }

    #[test]
    fn test_pong_response_json_structure() {
        let resp = ChatResponse::Pong;
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "pong");
    }

    // ── Edge case tests ─────────────────────────────────────────────────

    #[test]
    fn test_content_chunk_empty_text() {
        let resp = ChatResponse::ContentChunk {
            text: String::new(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        if let ChatResponse::ContentChunk { text } = deserialized {
            assert!(text.is_empty());
        } else {
            panic!("expected ContentChunk variant");
        }
    }

    #[test]
    fn test_content_chunk_ansi_text() {
        let resp = ChatResponse::ContentChunk {
            text: "\x1b[1mBold\x1b[0m text".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        if let ChatResponse::ContentChunk { text } = deserialized {
            assert_eq!(text, "\x1b[1mBold\x1b[0m text");
        } else {
            panic!("expected ContentChunk variant");
        }
    }

    #[test]
    fn test_error_empty_message() {
        let resp = ChatResponse::Error {
            message: String::new(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: ChatResponse = serde_json::from_slice(&json).unwrap();
        if let ChatResponse::Error { message } = deserialized {
            assert!(message.is_empty());
        } else {
            panic!("expected Error variant");
        }
    }

    #[test]
    fn test_stop_session_empty_agent_id() {
        let req = ChatRequest::StopSession {
            agent_id: String::new(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: ChatRequest = serde_json::from_slice(&json).unwrap();
        if let ChatRequest::StopSession { agent_id } = deserialized {
            assert!(agent_id.is_empty());
        } else {
            panic!("expected StopSession variant");
        }
    }
}
