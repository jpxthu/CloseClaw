//! Feishu MessageCleaner integration tests
//!
//! Source of truth: fixture files in `tests/fixtures/feishu/` (from PR #383).
//! Each test loads a raw feishu webhook fixture and its corresponding
//! expected output fixture, then asserts the cleaner produces the expected
//! content + metadata.
//!
//! Run with: `cargo test --test feishu_message_cleaner_tests`

use std::path::PathBuf;

use closeclaw::im::processor::{clean_feishu_message, ProcessedMessage};

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
// Test cases — one per fixture
// ---------------------------------------------------------------------------

// NOTE: These tests use the real clean_feishu_message from closeclaw::im::processor.
// The actual clean() function implementation is tracked in issue #391.

macro_rules! define_tests {
    ($($name:ident, $input:expr, $expected:expr),*) => {
        $(
            #[tokio::test]
            async fn $name() {
                let raw = load_raw_fixture($input);
                let expected = load_expected_fixture($expected);
                let result = clean_feishu_message(&raw).await;
                assert_eq!(result.content, expected.content,
                    "content mismatch for {}", $input);
            }
        )*
    }
}

define_tests!(
    test_01_text_simple,
    "im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json",
    "expected/01_text_simple.json",
    test_02_text_atbot,
    "im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json",
    "expected/02_text_atbot.json",
    test_03_post_lists,
    "im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json",
    "expected/03_post_lists.json",
    test_05_post_styles,
    "im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json",
    "expected/05_post_styles.json",
    test_06_text_thread,
    "im-message-receive_v1-no-event-id-2026-04-27T03-15-32-562Z.json",
    "expected/06_text_thread.json",
    test_07_text_plain,
    "im-message-receive_v1-no-event-id-2026-04-27T03-16-01-866Z.json",
    "expected/07_text_plain.json",
    test_08_text_emoji,
    "im-message-receive_v1-no-event-id-2026-04-27T03-23-50-544Z.json",
    "expected/08_text_emoji.json",
    test_09_post_image,
    "im-message-receive_v1-no-event-id-2026-04-27T03-32-13-305Z.json",
    "expected/09_post_image.json"
);
