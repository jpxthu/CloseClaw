//! Mock implementation of [`SessionLookup`] for unit tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use closeclaw_common::{PendingMessage, SessionLookup};

/// A simple in-memory mock of [`SessionLookup`] for permission crate tests.
///
/// Supports:
/// - Parent-child session relationships (`get_parent_of`)
/// - Session → agent_id mapping (`get_chat_id`)
/// - Collecting pushed pending messages (`push_pending_message`)
pub struct MockSessionLookup {
    /// child_session_id → parent_session_id
    parents: HashMap<String, String>,
    /// session_id → agent_id
    sessions: HashMap<String, String>,
    /// Collected pending messages: (session_id, message)
    pending_messages: Mutex<Vec<(String, PendingMessage)>>,
}

impl MockSessionLookup {
    /// Create a new empty mock.
    pub fn new() -> Self {
        Self {
            parents: HashMap::new(),
            sessions: HashMap::new(),
            pending_messages: Mutex::new(Vec::new()),
        }
    }

    /// Register a session mapping (session_id → agent_id).
    pub fn register_session(&mut self, session_id: &str, agent_id: &str) {
        self.sessions
            .insert(session_id.to_string(), agent_id.to_string());
    }

    /// Register a parent-child relationship.
    pub fn register_parent_child(&mut self, parent_session_id: &str, child_session_id: &str) {
        self.parents
            .insert(child_session_id.to_string(), parent_session_id.to_string());
    }

    /// Convenience: register session + parent-child in one call.
    pub fn register(
        &mut self,
        parent_session_id: &str,
        parent_agent_id: &str,
        child_session_id: &str,
        child_agent_id: &str,
    ) {
        self.register_session(parent_session_id, parent_agent_id);
        self.register_session(child_session_id, child_agent_id);
        self.register_parent_child(parent_session_id, child_session_id);
    }

    /// Return all pending messages that were pushed (for assertions).
    pub fn pending_messages(&self) -> Vec<(String, PendingMessage)> {
        self.pending_messages.lock().unwrap().clone()
    }
}

impl Default for MockSessionLookup {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionLookup for MockSessionLookup {
    async fn get_parent_of(&self, child_id: &str) -> Option<String> {
        self.parents.get(child_id).cloned()
    }

    async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        self.sessions.get(session_id).cloned()
    }

    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        self.pending_messages
            .lock()
            .unwrap()
            .push((session_id.to_string(), msg));
        Ok(())
    }
}
