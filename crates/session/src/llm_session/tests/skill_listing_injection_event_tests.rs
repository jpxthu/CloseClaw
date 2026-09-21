//! Tests for the `skill_listing_injection` tracing event.
//!
//! Verifies that `build_llm_messages_with_listing` emits a
//! `tracing::info!` event when a non-empty skill listing is injected,
//! and does NOT emit the event when listing is empty or None.
//!
//! Every test carries `#[serial_test::serial]` (issue #3102 mechanism):
//! installing the log-capture subscriber registers the info-level
//! callsites in the process-global tracing callsite-interest cache, and
//! concurrent registration on the same callsite can drop events (it
//! emptied a capture buffer in issue #3102). Serialising the module
//! removes the race at negligible cost — see `capture_logs` in the
//! parent module for the shared helper.

use super::super::*;
use super::capture_logs;
use closeclaw_common::SkillListingProvider;
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stub SkillListingProvider that returns a fixed listing.
struct StubListingProvider {
    listing: String,
}

impl SkillListingProvider for StubListingProvider {
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.listing.clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.listing.clone()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        Vec::new()
    }
}

/// Create a test session with a skill listing provider injected.
fn session_with_provider(listing: &str) -> ConversationSession {
    let mut session = ConversationSession::new(
        "test-skill-listing".into(),
        "test-model".into(),
        PathBuf::from("/tmp"),
    );
    let provider: Arc<dyn SkillListingProvider> = Arc::new(StubListingProvider {
        listing: listing.to_string(),
    });
    session.skill_listing_provider = Some(provider);
    session
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Non-empty skill listing injection emits `skill_listing_injection` event.
///
/// Verifies:
/// - `build_llm_messages_with_listing` with a non-empty listing emits
///   a tracing::info! with event="skill_listing_injection"
/// - The event contains entry_count and first_entry fields
#[serial_test::serial]
#[tokio::test]
async fn test_skill_listing_injection_emits_event() {
    let session = session_with_provider("- **skill-a**: desc-a\n- **skill-b**: desc-b");

    // `build_llm_messages_with_listing` is a private method on
    // ConversationSession; this test module lives inside the same crate
    // (a descendant of `llm_session`), so it is reachable directly.
    let listing = Some("- **skill-a**: desc-a\n- **skill-b**: desc-b".to_string());
    let (_messages, output) = capture_logs(
        || session.build_llm_messages_with_listing("hello", listing),
        tracing::Level::INFO,
    );

    assert!(
        output.contains("skill_listing_injection"),
        "expected skill_listing_injection event, got: {output}"
    );
    assert!(
        output.contains("entry_count"),
        "expected entry_count field in event, got: {output}"
    );
    assert!(
        output.contains("first_entry"),
        "expected first_entry field in event, got: {output}"
    );
}

/// Empty skill listing does NOT emit the `skill_listing_injection` event.
#[serial_test::serial]
#[tokio::test]
async fn test_empty_listing_no_event() {
    let session = session_with_provider("");

    let listing = Some(String::new());
    let (_messages, output) = capture_logs(
        || session.build_llm_messages_with_listing("hello", listing),
        tracing::Level::INFO,
    );

    assert!(
        !output.contains("skill_listing_injection"),
        "empty listing must NOT emit skill_listing_injection event, got: {output}"
    );
}

/// None skill listing does NOT emit the `skill_listing_injection` event.
#[serial_test::serial]
#[tokio::test]
async fn test_none_listing_no_event() {
    let session = session_with_provider("some listing content");

    let (_messages, output) = capture_logs(
        || session.build_llm_messages_with_listing("hello", None),
        tracing::Level::INFO,
    );

    assert!(
        !output.contains("skill_listing_injection"),
        "None listing must NOT emit skill_listing_injection event, got: {output}"
    );
}

/// Single-entry listing emits event with entry_count=1.
#[serial_test::serial]
#[tokio::test]
async fn test_single_entry_listing_event_fields() {
    let session = session_with_provider("- **solo-skill**: only one");

    let listing = Some("- **solo-skill**: only one".to_string());
    let (_messages, output) = capture_logs(
        || session.build_llm_messages_with_listing("hello", listing),
        tracing::Level::INFO,
    );

    assert!(output.contains("skill_listing_injection"));
    assert!(output.contains("entry_count=1"));
    // first_entry field exists (value format depends on tracing Debug impl)
    assert!(output.contains("first_entry"));
}

/// Listing with blank lines: only non-empty lines counted.
#[serial_test::serial]
#[tokio::test]
async fn test_listing_with_blank_lines_count() {
    let session = session_with_provider("a\n\nb");

    let listing = Some("a\n\nb".to_string());
    let (_messages, output) = capture_logs(
        || session.build_llm_messages_with_listing("hello", listing),
        tracing::Level::INFO,
    );

    assert!(output.contains("skill_listing_injection"));
    // entry_count filters out empty lines, so 2 non-empty lines.
    assert!(output.contains("entry_count=2"));
    // first_entry field exists (value depends on tracing Debug format)
    assert!(output.contains("first_entry"));
}
