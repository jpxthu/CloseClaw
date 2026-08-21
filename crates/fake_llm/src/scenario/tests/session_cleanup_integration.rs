//! Tests for session cleanup integration into ScenarioEngine (Step 1.4).

use std::time::{Duration, Instant};

use super::*;
use crate::scenario::session;
use crate::types::ProtocolKind;

/// After CLEANUP_INTERVAL (100) `decide()` calls, expired sessions
/// should be removed. We inject an expired session via
/// `cleanup_expired_at` and verify cleanup removes it.
#[test]
fn cleanup_triggers_after_interval_and_removes_expired() {
    // Create a scenario with many turns so we can call decide()
    // 100+ times without hitting turn overflow.
    let mut turns: Vec<TurnResponse> = Vec::new();
    for i in 0..200 {
        turns.push(text_turn(&format!("turn-{}", i)));
    }
    let scenario = ScenarioDeclaration {
        name: "long-scenario".to_string(),
        match_: None,
        turns,
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // Session A: create with message "session-a" at the start.
    let feat_a = features("gpt-4", "session-a");
    let _ = engine.decide(&feat_a);
    assert!(engine.sessions.active_session_count() >= 1);

    // Inject an expired session directly into the tracker with a
    // `last_active` that is older than SESSION_TTL.
    let expired_key = SessionTracker::compute_history_key(&["expired-session".to_string()]);
    let expired_entry = session::SessionEntry {
        history: vec!["expired-session".to_string()],
        turn: 0,
        last_active: Instant::now() - Duration::from_secs(1801),
    };
    engine
        .sessions
        .sessions
        .entry("long-scenario".to_string())
        .or_default()
        .insert(expired_key, expired_entry);
    // Confirm expired session exists.
    assert!(engine.sessions.active_session_count() >= 2);

    // Drive 99 more requests (different messages each) to reach
    // CLEANUP_INTERVAL without triggering cleanup yet.
    for i in 1..100 {
        let feat = RequestFeatures {
            model: "gpt-4".to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: vec![MessageEntry {
                role: "user".to_string(),
                content: format!("other-{}", i),
            }],
            tools: vec![],
            protocol: ProtocolKind::OpenAi,
        };
        let _ = engine.decide(&feat);
    }
    // After 100 calls, cleanup has been triggered once. The expired
    // session should have been removed; the fresh sessions remain.
    let count = engine.sessions.active_session_count();
    // 100 fresh sessions (one per call) + 0 expired = 100
    assert_eq!(count, 100);
    // Verify the expired session was removed.
    assert_eq!(
        engine.sessions.sessions.get("long-scenario").unwrap().len(),
        100
    );
    // The request_count should be 100 after 100 decide() calls.
    assert_eq!(engine.request_count, 100);
}

/// Verify that the normal request flow is unaffected by cleanup
/// integration — sessions advance correctly across the cleanup
/// boundary.
#[test]
fn normal_flow_unaffected_by_cleanup() {
    let mut turns: Vec<TurnResponse> = Vec::new();
    for i in 0..300 {
        turns.push(text_turn(&format!("reply-{}", i)));
    }
    let scenario = ScenarioDeclaration {
        name: "flow-test".to_string(),
        match_: None,
        turns,
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // Start a session.
    let feat1 = features("gpt-4", "start");
    match engine.decide(&feat1) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("reply-0"));
        }
        _ => panic!("expected decision on first turn"),
    }

    // Drive past the cleanup interval boundary (100 calls).
    for i in 1..150 {
        let feat = RequestFeatures {
            model: "gpt-4".to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: vec![MessageEntry {
                role: "user".to_string(),
                content: format!("unrelated-{}", i),
            }],
            tools: vec![],
            protocol: ProtocolKind::OpenAi,
        };
        let _ = engine.decide(&feat);
    }

    // Now extend the original session — it should still be alive.
    let feat2 = RequestFeatures {
        model: "gpt-4".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".to_string(),
                content: "start".to_string(),
            },
            MessageEntry {
                role: "assistant".to_string(),
                content: "reply-0".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "continue".to_string(),
            },
        ],
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    };
    match engine.decide(&feat2) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("reply-1"));
        }
        _ => panic!("expected decision on second turn"),
    }

    // Verify request_count is correct.
    assert_eq!(engine.request_count, 151);
}
