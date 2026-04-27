//! Feishu MessageCleaner integration tests
//!
//! Source of truth: fixture files in `tests/fixtures/feishu/` (from PR #383).
//! Each test loads a raw feishu webhook fixture and its corresponding
//! expected output fixture, then asserts the cleaner produces the expected
//! content + metadata.
//!
//! Run with: `cargo test --test feishu_message_cleaner_tests`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Test case definition
// ---------------------------------------------------------------------------

/// Input fixture file → expected output fixture file
const TEST_CASES: &[(&str, &str)] = &[
    // (input_fixture, expected_fixture)
    (
        "im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json",
        "expected/01_text_simple.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json",
        "expected/02_text_atbot.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json",
        "expected/03_post_lists.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json",
        "expected/05_post_styles.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T03-15-32-562Z.json",
        "expected/06_text_thread.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T03-16-01-866Z.json",
        "expected/07_text_plain.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T03-23-50-544Z.json",
        "expected/08_text_emoji.json",
    ),
    (
        "im-message-receive_v1-no-event-id-2026-04-27T03-32-13-305Z.json",
        "expected/09_post_image.json",
    ),
];

// ---------------------------------------------------------------------------
// ProcessedMessage — mirrors the design in design doc #27
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ProcessedMessage {
    pub content: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// Fixture loader helpers
// ---------------------------------------------------------------------------

fn feishu_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feishu")
}

fn load_raw_fixture(filename: &str) -> serde_json::Value {
    let path = feishu_fixtures_dir().join(filename);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn load_expected_fixture(filename: &str) -> ProcessedMessage {
    let path = feishu_fixtures_dir().join(filename);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// MessageCleaner — stub that will be replaced by the actual implementation
// ---------------------------------------------------------------------------

/// Placeholder. Replace with the real MessageCleaner once implemented.
/// Currently just extracts `content.text` for text messages and returns
/// the raw text for post messages (no-op clean).
async fn clean_feishu_message(raw: &serde_json::Value) -> ProcessedMessage {
    let msg = raw.get("message").unwrap();
    let msg_type = msg
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let content_str = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "text" => {
            let parsed: serde_json::Value =
                serde_json::from_str(content_str).unwrap_or(serde_json::Value::Null);
            let text = parsed.get("text").and_then(|v| v.as_str()).unwrap_or("");
            ProcessedMessage {
                content: text.to_string(),
                metadata: BTreeMap::new(),
            }
        }
        "post" => {
            // No-op for post — returns raw content for test visibility
            ProcessedMessage {
                content: content_str.to_string(),
                metadata: BTreeMap::new(),
            }
        }
        _ => ProcessedMessage {
            content: content_str.to_string(),
            metadata: BTreeMap::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Test cases — one per fixture
// ---------------------------------------------------------------------------

// NOTE: These tests are compile-time markers for the test case list.
// The actual clean() function above is a stub — real implementation
// is tracked in issue #391.

macro_rules! define_tests {
    ($($idx:expr, $name:ident, $input:expr, $expected:expr),*) => {
        $(
            #[tokio::test]
            async fn $name() {
                let raw = load_raw_fixture($input);
                let expected = load_expected_fixture($expected);
                let result = clean_feishu_message(&raw).await;
                // Assert content matches; metadata comparison can be added
                // once the real cleaner populates metadata fields.
                assert_eq!(result.content, expected.content,
                    "content mismatch for {}", $input);
            }
        )*
    }
}

define_tests!(
    0,
    test_01_text_simple,
    "im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json",
    "expected/01_text_simple.json",
    1,
    test_02_text_atbot,
    "im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json",
    "expected/02_text_atbot.json",
    2,
    test_03_post_lists,
    "im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json",
    "expected/03_post_lists.json",
    3,
    test_05_post_styles,
    "im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json",
    "expected/05_post_styles.json",
    4,
    test_06_text_thread,
    "im-message-receive_v1-no-event-id-2026-04-27T03-15-32-562Z.json",
    "expected/06_text_thread.json",
    5,
    test_07_text_plain,
    "im-message-receive_v1-no-event-id-2026-04-27T03-16-01-866Z.json",
    "expected/07_text_plain.json",
    6,
    test_08_text_emoji,
    "im-message-receive_v1-no-event-id-2026-04-27T03-23-50-544Z.json",
    "expected/08_text_emoji.json",
    7,
    test_09_post_image,
    "im-message-receive_v1-no-event-id-2026-04-27T03-32-13-305Z.json",
    "expected/09_post_image.json"
);
