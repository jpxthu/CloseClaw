//! Inbox Module - Multi-Agent Communication with Persistence and Retry
//!
//! Implements reliable message delivery between agents with:
//! - Exponential backoff retry
//! - Dead letter handling
//! - Jitter to prevent thundering herd
//! - Stats and monitoring

pub mod manager;
pub mod tests;
pub mod types;

pub use manager::InboxManager;
pub use types::{
    CommStats, DeadLetterRecord, InboxConfig, InboxMessage, MessageStatus, MessageType,
};
