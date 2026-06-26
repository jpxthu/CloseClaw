use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Reference to a media attachment (image, file, audio) in a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    /// Platform-specific media key for downloading the resource.
    pub key: String,
    /// URL pointing to the media resource.
    pub url: String,
}

/// Quoted/replied-to message embedded in an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotedMessage {
    /// Text content of the quoted message.
    pub content: String,
    /// Sender ID of the quoted message, if available.
    pub sender_id: Option<String>,
}

// ---------------------------------------------------------------------------
// URL normalization
// ---------------------------------------------------------------------------

/// Adds `https://` prefix to bare URLs that lack an http/https scheme.
///
/// Skips URLs already inside markdown link syntax `[text](url)`.
pub fn normalize_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = text.len();
    let mut i = 0;

    while i < len {
        // Skip non-ASCII bytes (multi-byte UTF-8) — just copy them as a string slice
        if !bytes[i].is_ascii() {
            let start = i;
            i += 1;
            while i < len && !bytes[i].is_ascii() {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }

        // Skip markdown links [text](url)
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < len && bytes[j] != b']' {
                j += 1;
            }
            if j < len && j + 1 < len && bytes[j + 1] == b'(' {
                let mut k = j + 1;
                while k < len && bytes[k] != b')' {
                    k += 1;
                }
                out.push_str(&text[i..=k]);
                i = k + 1;
                continue;
            }
            out.push('[');
            i += 1;
            continue;
        }

        if i + 4 <= len && &text[i..i + 4] == "www." {
            out.push_str("https://www.");
            i += 4;
            while i < len
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'"'
                && bytes[i] != b'\''
                && bytes[i] != b')'
                && bytes[i] != b']'
            {
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }

        let preceded_by_scheme =
            i >= 3 && bytes[i - 3] == b':' && bytes[i - 2] == b'/' && bytes[i - 1] == b'/';
        if !preceded_by_scheme
            && i > 0
            && !bytes[i - 1].is_ascii_alphanumeric()
            && i < len
            && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'.')
        {
            let start = i;
            let mut j = i;
            while j < len
                && !bytes[j].is_ascii_whitespace()
                && bytes[j] != b'"'
                && bytes[j] != b'\''
                && bytes[j] != b'<'
                && bytes[j] != b')'
                && bytes[j] != b']'
            {
                j += 1;
            }
            let token = &text[start..j];

            if token.contains('.')
                && !token.starts_with("http://")
                && !token.starts_with("https://")
                && !token.starts_with("ftp://")
                && !token.starts_with("file://")
            {
                out.push_str("https://");
                out.push_str(token);
                i = j;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Code block language hint
// ---------------------------------------------------------------------------

/// Adds ` ```text` language hint to code blocks that lack a language tag.
pub fn add_code_block_language_hint(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\`\`\`)([^\w\n]|$)").unwrap());
    RE.replace_all(text, "```text$1").to_string()
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Default message type when not specified.
fn default_message_type() -> String {
    "text".to_string()
}

/// Normalized inbound message produced by IM platform adapters.
///
/// This is the unified intermediate structure across all messaging platforms,
/// shielding platform-specific differences from the Processor Chain and Gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedMessage {
    /// Platform identifier, e.g. `"feishu"`, `"discord"`.
    pub platform: String,

    /// Sender's platform-specific user ID.
    pub sender_id: String,

    /// Peer ID — a `chat_id` for group chats, or the other party's user ID for
    /// private chats.
    pub peer_id: String,

    /// Message text content.
    pub content: String,

    /// Message send time as a Unix timestamp (milliseconds since epoch).
    pub timestamp: i64,

    /// Message type (`"text"`, `"image"`, `"file"`, `"audio"`, etc.).
    ///
    /// Defaults to `"text"` when the platform does not specify a type.
    #[serde(default = "default_message_type")]
    pub message_type: String,

    /// Media attachment references (images, files, audio).
    #[serde(default)]
    pub media_refs: Vec<MediaRef>,

    /// Quoted/replied-to message, if present. At most one level of nesting.
    pub quoted_message: Option<QuotedMessage>,

    /// Optional thread/topic ID. Used for定向 replies on platforms that support
    /// threads; does **not** participate in session key calculation.
    pub thread_id: Option<String>,

    /// Optional tenant/account identifier for multi-tenant session isolation.
    pub account_id: Option<String>,

    /// Whether this message is a card action (e.g. button click).
    ///
    /// `Some(true)` when the inbound event is a card action trigger;
    /// `None` (default) for regular text messages.
    #[serde(default)]
    pub card_action: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::NormalizedMessage;

    #[test]
    fn test_roundtrip_with_all_fields() {
        let msg = NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_abc123".to_string(),
            peer_id: "oc_group456".to_string(),
            content: "Hello, world!".to_string(),
            timestamp: 1_700_000_000_000,
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: Some("omt_thread789".to_string()),
            account_id: Some("tenant_001".to_string()),
            card_action: None,
        };

        let json = serde_json::to_string(&msg).expect("serialization failed");
        let deserialized: NormalizedMessage =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.platform, "feishu");
        assert_eq!(deserialized.sender_id, "ou_abc123");
        assert_eq!(deserialized.peer_id, "oc_group456");
        assert_eq!(deserialized.content, "Hello, world!");
        assert_eq!(deserialized.timestamp, 1_700_000_000_000);
        assert_eq!(deserialized.message_type, "text");
        assert!(deserialized.media_refs.is_empty());
        assert!(deserialized.quoted_message.is_none());
        assert_eq!(deserialized.thread_id.as_deref(), Some("omt_thread789"));
        assert_eq!(deserialized.account_id.as_deref(), Some("tenant_001"));
        assert_eq!(deserialized.card_action, None);
    }

    #[test]
    fn test_roundtrip_with_none_optional_fields() {
        let msg = NormalizedMessage {
            platform: "discord".to_string(),
            sender_id: "user123".to_string(),
            peer_id: "dm456".to_string(),
            content: "Direct message".to_string(),
            timestamp: 1_700_000_000_000,
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
        };

        let json = serde_json::to_string(&msg).expect("serialization failed");
        let deserialized: NormalizedMessage =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.platform, "discord");
        assert_eq!(deserialized.sender_id, "user123");
        assert_eq!(deserialized.peer_id, "dm456");
        assert_eq!(deserialized.content, "Direct message");
        assert_eq!(deserialized.timestamp, 1_700_000_000_000);
        assert_eq!(deserialized.message_type, "text");
        assert!(deserialized.media_refs.is_empty());
        assert!(deserialized.quoted_message.is_none());
        assert!(deserialized.thread_id.is_none());
        assert!(deserialized.account_id.is_none());
        assert_eq!(deserialized.card_action, None);
    }

    #[test]
    fn test_json_field_names_are_correct() {
        let msg = NormalizedMessage {
            platform: "test".to_string(),
            sender_id: "s".to_string(),
            peer_id: "p".to_string(),
            content: "c".to_string(),
            timestamp: 42,
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
        };

        let json = serde_json::to_value(&msg).expect("serialization to value failed");
        let obj = json.as_object().expect("expected JSON object");

        assert!(obj.contains_key("platform"));
        assert!(obj.contains_key("sender_id"));
        assert!(obj.contains_key("peer_id"));
        assert!(obj.contains_key("content"));
        assert!(obj.contains_key("timestamp"));
        assert!(obj.contains_key("message_type"));
        assert!(obj.contains_key("media_refs"));
        assert!(obj.contains_key("quoted_message"));
        assert!(obj.contains_key("thread_id"));
        assert!(obj.contains_key("account_id"));
        assert!(obj.contains_key("card_action"));
        assert_eq!(obj.len(), 11);
    }
}
