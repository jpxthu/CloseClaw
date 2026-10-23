//! Shared test utilities for cli terminal-channel tests.
//!
//! Single source of truth for constructing terminal [`NormalizedMessage`]s
//! in tests (chat.md §入站：TerminalAdapter field values, with deterministic
//! test values where the production path is non-deterministic).
//!
//! `terminal_tests.rs` deliberately does NOT use this helper: its
//! `NormalizedMessage` literals are intentionally differentiated per test
//! case. Do not "fix" that by converging them here.

use closeclaw_common::NormalizedMessage;

/// Build the canonical terminal-channel [`NormalizedMessage`] for tests.
///
/// Field values per chat.md §入站：TerminalAdapter (platform = "terminal",
/// account_id = "owner", message_type = text) plus deterministic test
/// values: `peer_id` = "cli", `sender_id` = "owner", `timestamp` = 0.
/// Unlike `TerminalAdapter::make_message` (production), this does not read
/// the OS uid or the wall clock, keeping tests deterministic.
pub(crate) fn terminal_msg(content: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "owner".to_string(),
        peer_id: "cli".to_string(),
        content: content.to_string(),
        timestamp: 0,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "owner".to_string(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: String::new(),
        reply_ref: None,
        unavailable_media: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::MessageType;

    /// Helper contract: every field of [`terminal_msg`] matches the
    /// helper's documented values — chat.md terminal channel values
    /// (platform, account_id, message_type) plus deterministic test
    /// values (peer_id, sender_id, timestamp) and pass-through content.
    #[test]
    fn test_terminal_msg_helper_contract() {
        let msg = terminal_msg("x");
        assert_eq!(msg.platform, "terminal");
        assert_eq!(msg.sender_id, "owner");
        assert_eq!(msg.peer_id, "cli");
        assert_eq!(msg.account_id, "owner");
        assert_eq!(msg.content, "x");
        assert_eq!(msg.timestamp, 0);
        assert_eq!(msg.message_type, MessageType::Text);
        assert!(msg.thread_id.is_none());
        assert!(msg.reply_ref.is_none());
        assert!(msg.media_refs.is_empty());
        assert!(msg.unavailable_media.is_empty());
        assert_eq!(msg.chat_name, String::new());
        assert_eq!(msg.trace_id, String::new());
        assert_eq!(msg.message_id, String::new());
    }
}
