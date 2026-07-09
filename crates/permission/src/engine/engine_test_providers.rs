//! Test helpers: HashMap-based AgentPermissionProvider for unit tests.

use std::collections::HashMap;

use closeclaw_config::agents::{AgentPermissionProvider, AgentPermissions};

/// A simple in-memory [`AgentPermissionProvider`] backed by a
/// `HashMap<String, AgentPermissions>`, useful in unit tests.
pub struct HashMapProvider {
    inner: HashMap<String, AgentPermissions>,
}

impl HashMapProvider {
    pub fn new(perms: HashMap<String, AgentPermissions>) -> Self {
        Self { inner: perms }
    }
}

impl AgentPermissionProvider for HashMapProvider {
    fn get(&self, agent_id: &str) -> Option<AgentPermissions> {
        self.inner.get(agent_id).cloned()
    }
}
