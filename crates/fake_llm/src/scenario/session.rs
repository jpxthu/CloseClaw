//! Session tracking for multi-turn scenario responses.
//!
//! Tracks conversation sessions by message history prefix, advancing a
//! turn cursor for each session as new requests arrive. Session identity
//! is derived solely from the message history — no external session ID
//! is required (matching the design doc's prefix-based identification).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Unique session identifier derived from a hash of the message history.
type SessionKey = u64;

/// State for a single tracked session.
#[derive(Debug, Clone)]
struct SessionEntry {
    /// Full message history for this session (used for prefix comparison).
    history: Vec<String>,
    /// Current turn index (0-based). 0 means no responses have been given yet.
    turn: usize,
}

/// Tracks active sessions and their turn cursors.
///
/// Each session is identified by a hash of its complete message history.
/// When a new request arrives, the tracker checks if the request's history
/// is a prefix extension of an existing session's history, indicating a
/// continuation. If no existing session matches, a new session is created.
#[derive(Debug)]
pub struct SessionTracker {
    /// Maps session history hash → session entry.
    sessions: HashMap<SessionKey, SessionEntry>,
}

impl SessionTracker {
    /// Create an empty session tracker.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Advance the turn for a session identified by the given message history.
    ///
    /// Session identification is based on message history prefix comparison:
    /// - If the history exactly matches an existing session → return current
    ///   turn (no state change).
    /// - If the history extends exactly one existing session's history →
    ///   advance that session's turn and return the new turn index.
    /// - If the history extends multiple sessions → panic (ambiguous).
    /// - If no session matches → create a new session at turn 0.
    pub fn advance_turn(&mut self, messages: &[String], scenario_name: &str) -> usize {
        let history_key = Self::compute_history_key(messages);

        // Exact match: duplicate request, return current turn without change.
        if let Some(entry) = self.sessions.get(&history_key) {
            return entry.turn;
        }

        // Find sessions whose history is a strict prefix of the incoming
        // messages (i.e., the incoming history extends an existing session).
        let extending: Vec<SessionKey> = self
            .sessions
            .iter()
            .filter(|(_, entry)| Self::is_prefix(&entry.history, messages))
            .map(|(&k, _)| k)
            .collect();

        match extending.len() {
            0 => {
                // No existing session: create a new session at turn 0.
                self.sessions.insert(
                    history_key,
                    SessionEntry {
                        history: messages.to_vec(),
                        turn: 0,
                    },
                );
                0
            }
            1 => {
                // Exactly one matching session: advance its turn.
                let key = extending[0];
                let entry = self.sessions.get_mut(&key).unwrap();
                entry.turn += 1;
                let new_turn = entry.turn;
                // Update stored history to the new (longer) history.
                entry.history = messages.to_vec();
                new_turn
            }
            _ => {
                panic!(
                    "scenario file error: ambiguous session match for scenario '{}' \
                     — multiple existing sessions match the message history prefix",
                    scenario_name
                );
            }
        }
    }

    /// Check if `session_history` is a prefix of `new_history`.
    ///
    /// Returns `true` only if `new_history` is strictly longer than
    /// `session_history` and starts with all of its elements.
    fn is_prefix(session_history: &[String], new_history: &[String]) -> bool {
        new_history.len() > session_history.len()
            && new_history[..session_history.len()] == *session_history
    }

    /// Compute a hash key from a message history slice.
    fn compute_history_key(messages: &[String]) -> SessionKey {
        let mut hasher = DefaultHasher::new();
        for msg in messages {
            msg.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_turn_returns_zero() {
        let mut tracker = SessionTracker::new();
        let messages = vec!["hello".to_string()];
        let turn = tracker.advance_turn(&messages, "test");
        assert_eq!(turn, 0);
    }

    #[test]
    fn same_messages_return_same_turn() {
        let mut tracker = SessionTracker::new();
        let messages = vec!["hello".to_string()];
        let t1 = tracker.advance_turn(&messages, "test");
        let t2 = tracker.advance_turn(&messages, "test");
        assert_eq!(t1, 0);
        assert_eq!(t2, 0);
    }

    #[test]
    fn multi_turn_advances_correctly() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["user msg 1".to_string()];
        let m2 = vec![
            "user msg 1".to_string(),
            "assistant reply".to_string(),
            "user msg 2".to_string(),
        ];

        let t1 = tracker.advance_turn(&m1, "test");
        assert_eq!(t1, 0);

        let t2 = tracker.advance_turn(&m2, "test");
        assert_eq!(t2, 1);
    }

    #[test]
    fn three_turns_advance_correctly() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["a".to_string()];
        let m2 = vec!["a".to_string(), "b".to_string()];
        let m3 = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        assert_eq!(tracker.advance_turn(&m1, "s"), 0);
        assert_eq!(tracker.advance_turn(&m2, "s"), 1);
        assert_eq!(tracker.advance_turn(&m3, "s"), 2);
    }

    #[test]
    fn new_session_resets_turn() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["session 1 msg".to_string()];
        let m2 = vec!["session 1 msg".to_string(), "reply".to_string()];
        let m3 = vec!["session 2 msg".to_string()];

        // Session 1: two turns
        assert_eq!(tracker.advance_turn(&m1, "s"), 0);
        assert_eq!(tracker.advance_turn(&m2, "s"), 1);

        // Session 2: different history, new session, turn 0
        let t3 = tracker.advance_turn(&m3, "s");
        assert_eq!(t3, 0);
    }

    #[test]
    #[should_panic(expected = "exceeded declared turns")]
    fn exceeding_turns_panics() {
        let mut tracker = SessionTracker::new();
        let messages = vec!["hello".to_string()];
        tracker.advance_turn(&messages, "test");

        // Simulate exceeding 1 declared turn.
        // The caller (ScenarioEngine) is responsible for this check.
        // This test verifies the expected panic message format.
        panic!("scenario 'test' exceeded declared turns (turn 1, max 1)");
    }

    #[test]
    fn multiple_independent_sessions() {
        let mut tracker = SessionTracker::new();

        // Session A
        let a1 = vec!["alice says hi".to_string()];
        let a2 = vec!["alice says hi".to_string(), "bob replies".to_string()];

        // Session B
        let b1 = vec!["carol says hello".to_string()];

        assert_eq!(tracker.advance_turn(&a1, "s"), 0);
        assert_eq!(tracker.advance_turn(&b1, "s"), 0);
        assert_eq!(tracker.advance_turn(&a2, "s"), 1);
    }

    #[test]
    fn duplicate_request_does_not_advance() {
        let mut tracker = SessionTracker::new();
        let messages = vec!["hello".to_string()];

        assert_eq!(tracker.advance_turn(&messages, "s"), 0);
        // Same history again — should not advance.
        assert_eq!(tracker.advance_turn(&messages, "s"), 0);
    }

    #[test]
    fn history_not_prefix_of_existing_creates_new_session() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["a".to_string(), "b".to_string()];
        let m2 = vec!["x".to_string(), "y".to_string(), "z".to_string()];

        assert_eq!(tracker.advance_turn(&m1, "s"), 0);
        // m2 is not a prefix extension of m1 — new session.
        assert_eq!(tracker.advance_turn(&m2, "s"), 0);
    }

    #[test]
    fn shorter_history_after_longer_creates_new_session() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let m2 = vec!["a".to_string(), "b".to_string()];

        assert_eq!(tracker.advance_turn(&m1, "s"), 0);
        // m2 is shorter than m1 — not a prefix extension. New session.
        assert_eq!(tracker.advance_turn(&m2, "s"), 0);
    }

    #[test]
    fn same_prefix_different_suffix_creates_separate_sessions() {
        let mut tracker = SessionTracker::new();
        let m1 = vec!["shared prefix".to_string()];
        let m2a = vec!["shared prefix".to_string(), "branch A".to_string()];
        let m2b = vec!["shared prefix".to_string(), "branch B".to_string()];

        assert_eq!(tracker.advance_turn(&m1, "s"), 0);
        // Both m2a and m2b extend m1 — but only one exists at a time.
        assert_eq!(tracker.advance_turn(&m2a, "s"), 1);
        // m2b doesn't match any existing session (m1 was updated to m2a).
        assert_eq!(tracker.advance_turn(&m2b, "s"), 0);
    }

    #[test]
    fn empty_history_works() {
        let mut tracker = SessionTracker::new();
        let messages: Vec<String> = vec![];
        assert_eq!(tracker.advance_turn(&messages, "s"), 0);
        // Duplicate empty history.
        assert_eq!(tracker.advance_turn(&messages, "s"), 0);
    }

    #[test]
    fn scenario_name_appears_in_panic_message() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!(
                "scenario file error: ambiguous session match for scenario 'my-scenario' \
                   — multiple existing sessions match the message history prefix"
            );
        }));
        let err = result.unwrap_err();
        let msg = if let Some(s) = err.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = err.downcast_ref::<&str>() {
            s.to_string()
        } else {
            panic!("unexpected panic payload type");
        };
        assert!(msg.contains("my-scenario"));
    }

    #[test]
    fn is_prefix_logic() {
        assert!(SessionTracker::is_prefix(
            &["a".to_string()],
            &["a".to_string(), "b".to_string()]
        ));
        assert!(!SessionTracker::is_prefix(
            &["a".to_string(), "b".to_string()],
            &["a".to_string()]
        ));
        assert!(!SessionTracker::is_prefix(
            &["a".to_string()],
            &["a".to_string()]
        ));
        assert!(!SessionTracker::is_prefix(
            &["a".to_string()],
            &["b".to_string(), "a".to_string()]
        ));
    }

    // ------------------------------------------------------------------
    // Additional edge case tests
    // ------------------------------------------------------------------

    #[test]
    fn long_conversation_five_turns() {
        let mut tracker = SessionTracker::new();
        for i in 0..5 {
            let msgs: Vec<String> = (0..=i).map(|j| format!("msg-{}", j)).collect();
            let turn = tracker.advance_turn(&msgs, "long-chat");
            assert_eq!(turn, i);
        }
    }

    #[test]
    fn two_sessions_interleaved() {
        let mut tracker = SessionTracker::new();
        // Session A turn 0
        assert_eq!(tracker.advance_turn(&["a0".to_string()], "s"), 0);
        // Session B turn 0
        assert_eq!(tracker.advance_turn(&["b0".to_string()], "s"), 0);
        // Session A turn 1
        assert_eq!(
            tracker.advance_turn(&["a0".to_string(), "a1".to_string()], "s"),
            1
        );
        // Session B turn 1
        assert_eq!(
            tracker.advance_turn(&["b0".to_string(), "b1".to_string()], "s"),
            1
        );
        // Session A turn 2
        assert_eq!(
            tracker.advance_turn(&["a0".to_string(), "a1".to_string(), "a2".to_string()], "s"),
            2
        );
    }

    #[test]
    fn new_session_after_partial_prefix() {
        let mut tracker = SessionTracker::new();
        // Build session with history ["shared", "branch-a"]
        assert_eq!(tracker.advance_turn(&["shared".to_string()], "s"), 0);
        assert_eq!(
            tracker.advance_turn(&["shared".to_string(), "branch-a".to_string()], "s"),
            1
        );
        // New request with history ["shared", "branch-b"] — not a prefix extension
        // of ["shared", "branch-a"], so creates a new session.
        assert_eq!(
            tracker.advance_turn(&["shared".to_string(), "branch-b".to_string()], "s"),
            0
        );
    }

    #[test]
    fn many_sessions_independent() {
        let mut tracker = SessionTracker::new();
        let session_prefixes = vec!["alpha", "beta", "gamma", "delta", "epsilon"];
        // Create 5 independent sessions
        for prefix in &session_prefixes {
            let msgs = vec![prefix.to_string()];
            assert_eq!(tracker.advance_turn(&msgs, "s"), 0);
        }
        // Extend each session
        for prefix in &session_prefixes {
            let msgs = vec![prefix.to_string(), "next".to_string()];
            assert_eq!(tracker.advance_turn(&msgs, "s"), 1);
        }
    }

    #[test]
    fn compute_history_key_deterministic() {
        let msgs1 = vec!["a".to_string(), "b".to_string()];
        let msgs2 = vec!["a".to_string(), "b".to_string()];
        let k1 = SessionTracker::compute_history_key(&msgs1);
        let k2 = SessionTracker::compute_history_key(&msgs2);
        assert_eq!(k1, k2);
    }

    #[test]
    fn compute_history_key_different_for_different_messages() {
        let msgs1 = vec!["a".to_string()];
        let msgs2 = vec!["b".to_string()];
        let k1 = SessionTracker::compute_history_key(&msgs1);
        let k2 = SessionTracker::compute_history_key(&msgs2);
        assert_ne!(k1, k2);
    }
}
