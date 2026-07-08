//! Session Mode query trait for cross-crate mode lookup.
//!
//! `SessionModeQuery` allows the permission engine to look up an agent's
//! current `SessionMode` without depending on the session crate directly.

use crate::session_mode::SessionMode;

/// Synchronous query interface: given an agent ID, return its current `SessionMode`.
///
/// Implementors bridge the session layer's mode state into any consumer
/// (e.g. the permission engine) without creating a hard dependency.
///
/// The lookup is expected to be fast (typically backed by an in-memory store).
pub trait SessionModeQuery: Send + Sync {
    /// Look up the session mode for the given agent.
    ///
    /// Returns `None` if the agent is unknown or has no active session.
    fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct MockSessionModeQuery {
        modes: HashMap<String, SessionMode>,
    }

    impl MockSessionModeQuery {
        fn new() -> Self {
            Self {
                modes: HashMap::new(),
            }
        }

        fn with_mode(mut self, agent_id: &str, mode: SessionMode) -> Self {
            self.modes.insert(agent_id.to_string(), mode);
            self
        }
    }

    impl SessionModeQuery for MockSessionModeQuery {
        fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode> {
            self.modes.get(agent_id).copied()
        }
    }

    #[test]
    fn test_known_agent_returns_mode() {
        let query = MockSessionModeQuery::new()
            .with_mode("agent-1", SessionMode::Plan)
            .with_mode("agent-2", SessionMode::Auto);
        assert_eq!(query.get_session_mode("agent-1"), Some(SessionMode::Plan));
        assert_eq!(query.get_session_mode("agent-2"), Some(SessionMode::Auto));
    }

    #[test]
    fn test_unknown_agent_returns_none() {
        let query = MockSessionModeQuery::new();
        assert_eq!(query.get_session_mode("no-such-agent"), None);
    }

    #[test]
    fn test_trait_object_dyn() {
        let query: Arc<dyn SessionModeQuery> =
            Arc::new(MockSessionModeQuery::new().with_mode("a", SessionMode::Normal));
        assert_eq!(query.get_session_mode("a"), Some(SessionMode::Normal));
    }
}
