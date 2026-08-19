//! Spawn tree: maintains parent-child relationships between sessions.
//!
//! Wraps the `children` lookup table (parent_session_id → child list)
//! and exposes the three query interfaces described in the design doc:
//! `list_children`, `list_descendants`, `get_parent`.
//!
//! This module is the authoritative source for runtime spawn tree
//! tracking. The Gateway's `SessionManager` holds a `SpawnTree`
//! instance behind an async `RwLock` and delegates all query and
//! mutation calls through it.

use std::collections::{HashMap, VecDeque};

use super::types::{ChildSessionInfo, ChildSessionStatus};

/// Spawn tree: maintains parent-child relationships between sessions.
pub struct SpawnTree {
    inner: HashMap<String, Vec<ChildSessionInfo>>,
}

impl SpawnTree {
    /// Create an empty spawn tree.
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    // ── Design doc query interfaces ────────────────────────────────

    /// List all direct children of a session.
    pub fn list_children(&self, session_id: &str) -> Vec<&ChildSessionInfo> {
        self.inner
            .get(session_id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// List all descendants of a session (recursive BFS).
    ///
    /// Returns session IDs in reverse BFS order (deepest first,
    /// shallowest last) so callers can process leaves before parents.
    pub fn list_descendants(&self, session_id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        if let Some(list) = self.inner.get(session_id) {
            for info in list {
                queue.push_back(info.session_id.clone());
            }
        }
        while let Some(current) = queue.pop_front() {
            result.push(current.clone());
            if let Some(list) = self.inner.get(&current) {
                for info in list {
                    queue.push_back(info.session_id.clone());
                }
            }
        }
        result.reverse();
        result
    }

    /// Get the parent session ID of a given session.
    pub fn get_parent(&self, session_id: &str) -> Option<String> {
        self.inner
            .values()
            .flatten()
            .find(|info| info.session_id == session_id)
            .map(|info| info.parent_session_id.clone())
    }

    /// Check if the tree is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Find a child session info by its session ID.
    pub fn find_child(&self, session_id: &str) -> Option<&ChildSessionInfo> {
        self.inner
            .values()
            .flatten()
            .find(|info| info.session_id == session_id)
    }

    /// Register a child session under its parent.
    pub fn register_child(&mut self, parent_id: &str, info: ChildSessionInfo) {
        self.inner
            .entry(parent_id.to_string())
            .or_default()
            .push(info);
    }

    /// Update the status of a child session by its session ID.
    ///
    /// Returns `true` if the child was found and updated, `false` otherwise.
    pub fn mark_child_status(&mut self, child_id: &str, status: ChildSessionStatus) -> bool {
        for list in self.inner.values_mut() {
            if let Some(info) = list.iter_mut().find(|i| i.session_id == child_id) {
                info.status = status;
                return true;
            }
        }
        false
    }

    /// Iterate all parent → children entries.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<ChildSessionInfo>)> {
        self.inner.iter()
    }

    /// Remove a direct child from its parent's list. Removes the
    /// parent entry entirely if the list becomes empty.
    pub fn remove_child(&mut self, parent_id: &str, child_id: &str) {
        if let Some(list) = self.inner.get_mut(parent_id) {
            list.retain(|info| info.session_id != child_id);
            if list.is_empty() {
                self.inner.remove(parent_id);
            }
        }
    }
}

impl SpawnTree {
    /// Remove completed/terminated leaf children under a parent.
    ///
    /// For each direct child of `parent_id` whose status is in the
    /// terminal set (Completed, Terminated), checks whether the child
    /// still has active descendants. If the subtree is fully terminal
    /// (no active descendants), the child is removed.
    ///
    /// Returns the list of removed session IDs (for caller logging).
    pub fn reclaim_completed(&mut self, parent_id: &str) -> Vec<String> {
        let mut reclaimed = Vec::new();

        // First pass: collect terminal child IDs that are safe to remove
        // (no active descendants in their subtree).
        let candidates: Vec<String> = self
            .inner
            .get(parent_id)
            .map(|children| {
                children
                    .iter()
                    .filter(|c| c.status.is_terminal())
                    .filter(|c| !self.has_active_descendants(&c.session_id))
                    .map(|c| c.session_id.clone())
                    .collect()
            })
            .unwrap_or_default();

        // Second pass: remove collected children.
        for child_id in candidates {
            self.remove_child(parent_id, &child_id);
            reclaimed.push(child_id);
        }

        reclaimed
    }

    /// Recursively check whether any descendant of `session_id` is active.
    fn has_active_descendants(&self, session_id: &str) -> bool {
        if let Some(children) = self.inner.get(session_id) {
            for child in children {
                if child.status == ChildSessionStatus::Active {
                    return true;
                }
                if self.has_active_descendants(&child.session_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Remove entries for descendant sessions from the tree.
    /// For each descendant, removes it from its parent's list and
    /// removes any sub-entries where it is itself a parent.
    pub fn remove_descendant_entries(&mut self, descendant_ids: &[String]) {
        for id in descendant_ids {
            let parent = self
                .inner
                .values_mut()
                .find(|list| list.iter().any(|info| info.session_id == *id));
            if let Some(list) = parent {
                list.retain(|info| info.session_id != *id);
            }
            self.inner.remove(id);
        }
    }
}

impl Default for SpawnTree {
    fn default() -> Self {
        Self::new()
    }
}
