//! Shared test utilities for cli tests.
//!
//! Single source of truth for constructing terminal [`NormalizedMessage`]s
//! in tests (chat.md §入站：TerminalAdapter field values, with deterministic
//! test values where the production path is non-deterministic), and for the
//! clap wrapper used to parse [`crate::args::ChatArgs`] in isolation.
//!
//! `terminal_tests.rs` deliberately does NOT use this helper: its
//! `NormalizedMessage` literals are intentionally differentiated per test
//! case. Do not "fix" that by converging them here.

use closeclaw_common::NormalizedMessage;

/// Wrapper to parse `ChatArgs` through clap in isolation (the `chat`
/// subcommand's argument set without the surrounding command). Shared by the
/// `args.rs` parsing tests and the config-wizard same-source invariant tests.
#[derive(clap::Parser, Debug)]
pub(crate) struct ChatArgsCli {
    #[command(flatten)]
    pub(crate) args: crate::args::ChatArgs,
}

/// Build the canonical terminal-channel [`NormalizedMessage`] for tests.
///
/// Field values per chat.md §入站：TerminalAdapter (platform = "terminal",
/// account_id = "owner", message_type = text) plus deterministic test
/// values: `peer_id` = "test-agent", `sender_id` = "owner", `timestamp` = 0.
/// Unlike `TerminalAdapter::make_message` (production), this does not read
/// the OS uid or the wall clock, keeping tests deterministic.
/// Use [`terminal_msg_with_peer`] to exercise peer_id pass-through.
pub(crate) fn terminal_msg(content: &str) -> NormalizedMessage {
    terminal_msg_with_peer(content, "test-agent")
}

/// [`terminal_msg`] with an explicit `peer_id`: production peer_id is the
/// `--agent-id` value (chat.md §入站：TerminalAdapter), never a fixed
/// constant, so tests that observe peer_id must pass a meaningful value.
pub(crate) fn terminal_msg_with_peer(content: &str, peer_id: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "owner".to_string(),
        peer_id: peer_id.to_string(),
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
    /// [`terminal_msg_with_peer`] overrides only peer_id; the other peer
    /// value comes from the explicit argument.
    #[test]
    fn test_terminal_msg_helper_contract() {
        let msg = terminal_msg("x");
        assert_eq!(msg.platform, "terminal");
        assert_eq!(msg.sender_id, "owner");
        assert_eq!(msg.peer_id, "test-agent");
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

        let with_peer = terminal_msg_with_peer("x", "agent-x");
        assert_eq!(with_peer.peer_id, "agent-x");
        assert_eq!(with_peer.platform, msg.platform);
        assert_eq!(with_peer.sender_id, msg.sender_id);
        assert_eq!(with_peer.account_id, msg.account_id);
        assert_eq!(with_peer.content, msg.content);
        assert_eq!(with_peer.timestamp, msg.timestamp);
        assert_eq!(with_peer.message_type, msg.message_type);
        assert_eq!(with_peer.thread_id, msg.thread_id);
        assert!(with_peer.reply_ref.is_none());
        assert!(with_peer.media_refs.is_empty());
        assert!(with_peer.unavailable_media.is_empty());
        assert_eq!(with_peer.chat_name, msg.chat_name);
        assert_eq!(with_peer.trace_id, msg.trace_id);
        assert_eq!(with_peer.message_id, msg.message_id);
    }
}
