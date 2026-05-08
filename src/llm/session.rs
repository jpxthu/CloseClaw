//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage` and `ChatSession` trait for managing conversation state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::llm::types::ContentBlock;

/// A single message in a conversation session.
///
/// Contains the role of the sender, content blocks, and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    /// Role of the message sender (e.g., "user", "assistant", "system").
    pub role: String,
    /// Ordered list of content blocks.
    pub content_blocks: Vec<ContentBlock>,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
}
