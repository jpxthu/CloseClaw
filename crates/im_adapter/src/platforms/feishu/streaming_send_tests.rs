//! Unit tests for [`FeishuPlugin::lock_streaming_state`].
//!
//! Covers the helper's behavior dimensions without touching the platform:
//! - the returned guard exposes the renderer's initial state
//! - mutations through the guard are observable after re-locking
//! - the lock can be re-acquired once the previous guard is dropped

use super::make_plugin;

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
