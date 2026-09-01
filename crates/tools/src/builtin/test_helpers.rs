//! Shared test helpers for builtin tool tests.

/// Create a minimal [`ToolContext`] for testing.
pub(crate) fn test_ctx() -> crate::ToolContext {
    crate::ToolContext {
        agent_id: "test-agent".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}
