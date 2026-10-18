//! Unit tests for [`FeishuPlugin::lock_streaming_state`] and
//! [`FeishuPlugin::accumulate_streaming_text`].
//!
//! Covers the helper's behavior dimensions without touching the platform:
//! - the returned guard exposes the renderer's initial state
//! - mutations through the guard are observable after re-locking
//! - the lock can be re-acquired once the previous guard is dropped
//! - text accumulation state transitions (Initialize / Update / Skip)

use std::time::Instant;

use super::make_plugin;
use super::streaming_send::StreamingAction;
use closeclaw_common::RenderedOutput;

#[test]
fn test_lock_streaming_state_guard_reads_initial_state() {
    let plugin = make_plugin("unused-cli");
    let state = plugin.lock_streaming_state();
    assert_eq!(state.state.card_id, None, "card_id should start as None");
    assert_eq!(state.state.sequence, 0, "sequence should start at 0");
    assert!(!state.state.is_active, "is_active should start false");
    assert!(
        state.state.last_update.is_none(),
        "last_update should start as None"
    );
    assert!(
        state.state.pending_text.is_empty(),
        "pending_text should start empty"
    );
}

#[test]
fn test_lock_streaming_state_mutation_visible_on_relock() {
    let plugin = make_plugin("unused-cli");
    {
        let mut state = plugin.lock_streaming_state();
        state.state.pending_text.push_str("hello");
        state.state.sequence = 2;
        state.state.card_id = Some("card_1".to_string());
    }
    let state = plugin.lock_streaming_state();
    assert_eq!(state.state.pending_text, "hello");
    assert_eq!(state.state.sequence, 2);
    assert_eq!(state.state.card_id.as_deref(), Some("card_1"));
}

#[test]
fn test_lock_streaming_state_reacquire_after_guard_drop() {
    let plugin = make_plugin("unused-cli");
    {
        let guard = plugin.lock_streaming_state();
        assert_eq!(guard.state.sequence, 0);
    }
    let mut again = plugin.lock_streaming_state();
    assert_eq!(again.state.sequence, 0);
    again.state.is_active = true;
    assert!(again.state.is_active);
}

/// Build a text [`RenderedOutput`] whose payload carries `content.text`,
/// the field read by `accumulate_streaming_text`.
fn text_output(content: &str) -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({
            "msg_type": "text",
            "content": { "text": content }
        }),
    }
}

#[test]
fn test_accumulate_streaming_text_initialize_on_first_non_empty_text() {
    let plugin = make_plugin("unused-cli");
    {
        let state = plugin.lock_streaming_state();
        assert_eq!(state.state.card_id, None, "card_id should start as None");
        assert!(
            state.state.last_update.is_none(),
            "last_update should start as None"
        );
    }
    let action = plugin.accumulate_streaming_text(&text_output("hello"));
    let (content, seq) = match action {
        StreamingAction::Initialize(content, seq) => (content, seq),
        StreamingAction::Skip => panic!("expected Initialize on first non-empty text, got Skip"),
        StreamingAction::Update(_, _, _) => {
            panic!("expected Initialize on first non-empty text, got Update")
        }
    };
    assert_eq!(content, "hello", "payload carries accumulated text");
    assert_eq!(seq, 1, "sequence increments from 0 to 1");
    let state = plugin.lock_streaming_state();
    assert_eq!(state.state.sequence, 1, "sequence state persisted");
    assert!(
        state.state.pending_text.is_empty(),
        "pending_text should be flushed into the action payload"
    );
    assert!(
        state.state.last_update.is_some(),
        "last_update should be stamped on flush"
    );
    assert_eq!(state.state.card_id, None, "card_id stays None");
}

#[test]
fn test_accumulate_streaming_text_update_when_card_id_preset() {
    let plugin = make_plugin("unused-cli");
    {
        let mut state = plugin.lock_streaming_state();
        // Update branch is keyed on card_id alone; is_active is deliberately
        // left false to prove it does not participate in the decision.
        state.state.card_id = Some("card_1".to_string());
    }
    let action = plugin.accumulate_streaming_text(&text_output("world"));
    let (card_id, content, seq) = match action {
        StreamingAction::Update(card_id, content, seq) => (card_id, content, seq),
        StreamingAction::Skip => panic!("expected Update when card_id is preset, got Skip"),
        StreamingAction::Initialize(_, _) => {
            panic!("expected Update when card_id is preset, got Initialize")
        }
    };
    assert_eq!(card_id, "card_1", "payload carries preset card_id");
    assert_eq!(content, "world", "payload carries accumulated text");
    assert_eq!(seq, 1, "sequence increments from 0 to 1");
    let state = plugin.lock_streaming_state();
    assert_eq!(state.state.sequence, 1, "sequence state persisted");
    assert!(
        state.state.pending_text.is_empty(),
        "pending_text should be flushed into the action payload"
    );
}

#[test]
fn test_accumulate_streaming_text_skip_within_throttle_window() {
    let plugin = make_plugin("unused-cli");
    {
        let mut state = plugin.lock_streaming_state();
        state.state.last_update = Some(Instant::now());
    }
    // Throttle basis: `should_update_now` requires 100ms since last_update
    // (cardkit_streaming.rs:75-80). Text is fed immediately after presetting
    // last_update, and the assertions below run without sleeps, so they
    // complete within the 100ms window.
    let action = plugin.accumulate_streaming_text(&text_output("later"));
    match action {
        StreamingAction::Skip => {}
        StreamingAction::Initialize(_, _) => {
            panic!("expected Skip within throttle window, got Initialize")
        }
        StreamingAction::Update(_, _, _) => {
            panic!("expected Skip within throttle window, got Update")
        }
    }
    let state = plugin.lock_streaming_state();
    assert_eq!(
        state.state.pending_text, "later",
        "throttled text should stay buffered for the next flush"
    );
    assert_eq!(state.state.sequence, 0, "sequence must not advance on Skip");
}

#[test]
fn test_accumulate_streaming_text_skip_on_empty_text() {
    let plugin = make_plugin("unused-cli");
    // last_update stays None so `should_update_now()` would allow a flush;
    // the Skip below is therefore deterministic and caused solely by the
    // empty-text guard, not by throttling.
    let action = plugin.accumulate_streaming_text(&text_output(""));
    match action {
        StreamingAction::Skip => {}
        StreamingAction::Initialize(_, _) => {
            panic!("expected Skip on empty text, got Initialize")
        }
        StreamingAction::Update(_, _, _) => {
            panic!("expected Skip on empty text, got Update")
        }
    }
    let state = plugin.lock_streaming_state();
    assert!(
        state.state.pending_text.is_empty(),
        "empty text must not be buffered"
    );
    assert_eq!(state.state.sequence, 0, "sequence must not advance on Skip");
    assert!(
        state.state.last_update.is_none(),
        "empty text must not stamp last_update"
    );
}
