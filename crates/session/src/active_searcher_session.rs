//! Active-searcher session identity and lifecycle tracking.
//!
//! Each active-searcher run is a lightweight logical sub-session
//! with a unique ID, parent session association, and explicit
//! lifecycle status. The tracker holds these in memory only
//! (no persistence / no checkpoint).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Maximum number of finished (non-Running) records kept in the tracker.
const TRACKER_CAPACITY: usize = 128;

/// Lifecycle status of a searcher session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearcherSessionStatus {
    /// Actively running (just spawned, search in progress).
    Running,
    /// Search completed and memory injection was written.
    Injected,
    /// Search completed but found nothing to inject.
    NoResult,
    /// Search was abandoned (typically due to timeout).
    Abandoned,
}

/// A lightweight identity record for one active-searcher run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearcherSession {
    /// Unique session ID (UUID v4).
    pub id: String,
    /// ID of the parent conversation session.
    pub parent_session_id: String,
    /// ID of the agent owning the session.
    pub agent_id: String,
    /// Role of the triggering message (`"user"` or `"assistant"`).
    pub trigger_role: String,
    /// Current lifecycle status.
    pub status: SearcherSessionStatus,
    /// When the session was created (unix timestamp millis).
    pub created_at: u64,
    /// When the session reached a terminal status (`None` if still Running).
    pub finished_at: Option<u64>,
}

impl SearcherSession {
    /// Current timestamp in milliseconds since the unix epoch.
    fn now_millis() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Create a new searcher session in `Running` status.
    fn new(parent_session_id: String, agent_id: String, trigger_role: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_session_id,
            agent_id,
            trigger_role,
            status: SearcherSessionStatus::Running,
            created_at: Self::now_millis(),
            finished_at: None,
        }
    }
}

/// In-memory tracker for active-searcher sessions.
///
/// Thread-safe via `std::sync::Mutex`. Only finished (terminal-status)
/// records are evicted when capacity is exceeded; `Running` records
/// are never evicted.
#[derive(Debug)]
pub struct SearcherSessionTracker {
    sessions: std::sync::Mutex<HashMap<String, SearcherSession>>,
}

impl Default for SearcherSessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SearcherSessionTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self {
            sessions: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Register a new searcher session and return its ID.
    ///
    /// The session starts in `Running` status.
    pub fn begin(
        &self,
        parent_session_id: String,
        agent_id: String,
        trigger_role: String,
    ) -> String {
        let session = SearcherSession::new(parent_session_id, agent_id, trigger_role);
        let id = session.id.clone();
        let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(id.clone(), session);
        self.evict_if_needed(&mut map);
        id
    }

    /// Mark a searcher session with a terminal status.
    ///
    /// If `session_id` is unknown or already terminal, this is a no-op.
    pub fn end(&self, session_id: &str, status: SearcherSessionStatus) {
        let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = map.get_mut(session_id) {
            // Only transition from Running to a terminal status.
            if session.status == SearcherSessionStatus::Running {
                session.status = status;
                session.finished_at = Some(SearcherSession::now_millis());
            }
        }
    }

    /// Look up a session by ID.
    pub fn get(&self, session_id: &str) -> Option<SearcherSession> {
        let map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id).cloned()
    }

    /// Return the current number of tracked sessions.
    pub fn len(&self) -> usize {
        let map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        map.len()
    }

    /// Return `true` if the tracker contains no sessions.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict the oldest finished records when over capacity.
    ///
    /// Only terminal-status records are eligible; `Running` records
    /// are never removed.
    fn evict_if_needed(&self, map: &mut HashMap<String, SearcherSession>) {
        if map.len() <= TRACKER_CAPACITY {
            return;
        }

        // Collect IDs of finished (non-Running) records.
        let mut finished: Vec<(String, u64)> = map
            .iter()
            .filter(|(_, s)| s.status != SearcherSessionStatus::Running)
            .map(|(id, s)| (id.clone(), s.finished_at.unwrap_or(s.created_at)))
            .collect();

        // Sort oldest first.
        finished.sort_by_key(|(_, ts)| *ts);

        // Remove oldest until back at capacity.
        let excess = map.len() - TRACKER_CAPACITY;
        for (id, _) in finished.into_iter().take(excess) {
            map.remove(&id);
        }
    }
}
