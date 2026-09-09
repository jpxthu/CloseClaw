//! Tests for the `skill_listing_injection` tracing event.
//!
//! Verifies that `build_llm_messages_with_listing` emits a
//! `tracing::info!` event when a non-empty skill listing is injected,
//! and does NOT emit the event when listing is empty or None.

use super::super::*;
use closeclaw_common::SkillListingProvider;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;

// ---------------------------------------------------------------------------
// Log-capture Layer — captures info+ log output into a shared buffer
// ---------------------------------------------------------------------------

struct CaptureLayer {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> tracing_subscriber::Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MsgVisitor {
            message: String::new(),
            fields: String::new(),
        };
        event.record(&mut visitor);

        let meta = event.metadata();
        let level = meta.level();
        let target = meta.target();

        let mut output = format!("[{level}] {target}: {}", visitor.message);
        if !visitor.fields.is_empty() {
            output.push_str(&format!(" {}", visitor.fields));
        }
        output.push('\n');

        if let Ok(mut buf) = self.buf.lock() {
            use std::io::Write;
            let _ = buf.write_all(output.as_bytes());
        }
    }
}

struct MsgVisitor {
    message: String,
    fields: String,
}
impl tracing::field::Visit for MsgVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            self.fields.push_str(&format!("{}={value:?}", field.name()));
        }
    }
}

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
#[tokio::test]
async fn test_skill_listing_injection_emits_event() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = CaptureLayer { buf: buf.clone() };
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let session = session_with_provider("- **skill-a**: desc-a\n- **skill-b**: desc-b");

    // build_llm_messages_with_listing is private; exercise it through
    // the message assembly: inject a skill listing via prepare + build.
    // Since build_llm_messages_with_listing is pub(crate), we call it
    // via the public invoke_llm path with a mock LlmCaller.
    // Alternatively, we test the tracing event directly via the internal
    // method by adding a test within the session crate.

    // We can't directly call build_llm_messages_with_listing from here
    // since it's pub(crate). Instead, test the public path:
    // invoke_llm which calls build_llm_messages_with_listing internally.
    // But invoke_llm requires a LlmCaller. Let's use the simpler approach:
    // just verify the event is emitted through the tracing capture.

    // Actually, since we're in the session crate's test module (tests/),
    // and build_llm_messages_with_listing is pub(crate), we CAN call it.
    // But we need to get to it. The session_llm module defines it as
    // `fn build_llm_messages_with_listing(...)` — it's a private fn on
    // ConversationSession. We can call it from within the crate's test
    // code because the tests module is a child of the parent module.

    // Since this test file is `mod skill_listing_injection_event_tests`
    // inside `mod tests` which is `mod tests` inside `mod llm_session`,
    // and build_llm_messages_with_listing is a private method on
    // ConversationSession, we need to use `invoke_llm` or access via
    // the session's internal test helpers.

    // Use a simpler approach: test via prepare_turn_skill_listing + the
    // build_llm_messages_with_listing that is called internally. Since
    // build_llm_messages_with_listing is a private method, we test
    // the tracing output via the public invoke_llm path with a mock.

    // Since this is a pub(crate) fn and we're in the same crate,
    // we can access it. But we need to call it as a method on session.

    // The method is defined as: fn build_llm_messages_with_listing(&self, ...)
    // on ConversationSession. Since we're in a test module that's a
    // descendant of the same crate, we can call it.
    let listing = Some("- **skill-a**: desc-a\n- **skill-b**: desc-b".to_string());
    let _messages = session.build_llm_messages_with_listing("hello", listing);

    let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
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
#[tokio::test]
async fn test_empty_listing_no_event() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = CaptureLayer { buf: buf.clone() };
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let session = session_with_provider("");

    let listing = Some(String::new());
    let _messages = session.build_llm_messages_with_listing("hello", listing);

    let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        !output.contains("skill_listing_injection"),
        "empty listing must NOT emit skill_listing_injection event, got: {output}"
    );
}

/// None skill listing does NOT emit the `skill_listing_injection` event.
#[tokio::test]
async fn test_none_listing_no_event() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = CaptureLayer { buf: buf.clone() };
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let session = session_with_provider("some listing content");

    let _messages = session.build_llm_messages_with_listing("hello", None);

    let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        !output.contains("skill_listing_injection"),
        "None listing must NOT emit skill_listing_injection event, got: {output}"
    );
}

/// Single-entry listing emits event with entry_count=1.
#[tokio::test]
async fn test_single_entry_listing_event_fields() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = CaptureLayer { buf: buf.clone() };
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let session = session_with_provider("- **solo-skill**: only one");

    let listing = Some("- **solo-skill**: only one".to_string());
    let _messages = session.build_llm_messages_with_listing("hello", listing);

    let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(output.contains("skill_listing_injection"));
    assert!(output.contains("entry_count=1"));
    // first_entry field exists (value format depends on tracing Debug impl)
    assert!(output.contains("first_entry"));
}

/// Listing with blank lines: only non-empty lines counted.
#[tokio::test]
async fn test_listing_with_blank_lines_count() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = CaptureLayer { buf: buf.clone() };
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let session = session_with_provider("a\n\nb");

    let listing = Some("a\n\nb".to_string());
    let _messages = session.build_llm_messages_with_listing("hello", listing);

    let output = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(output.contains("skill_listing_injection"));
    // entry_count filters out empty lines, so 2 non-empty lines.
    assert!(output.contains("entry_count=2"));
    // first_entry field exists (value depends on tracing Debug format)
    assert!(output.contains("first_entry"));
}
