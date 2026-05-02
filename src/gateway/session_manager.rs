//! SessionManager - extracted session management from Gateway
//!
//! Responsible for session lifecycle: lookup, creation, restoration.

use crate::gateway::{DmScope, GatewayConfig, Message, Session};
use crate::im::processor::ProcessError;
use crate::im::IMAdapter;
use crate::session::persistence::{PersistenceService, SessionStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

/// SessionManager holds all session state previously belonging to Gateway.
/// It provides find_or_create to lookup or create a session by channel + message.
pub struct SessionManager {
    /// Active sessions: session_id -> Session
    sessions: RwLock<HashMap<String, Session>>,
    /// Persistence backend (for archived session restoration)
    storage: RwLock<Option<Arc<dyn PersistenceService>>>,
    /// DM scope policy (determines how session keys are computed)
    dm_scope: DmScope,
    /// IM adapters for sending notifications during restoration
    adapters: RwLock<HashMap<String, Arc<dyn IMAdapter>>>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("dm_scope", &self.dm_scope)
            .finish_non_exhaustive()
    }
}

impl SessionManager {
    /// Create a new SessionManager with the given config and optional storage.
    pub fn new(config: &GatewayConfig, storage: Option<Arc<dyn PersistenceService>>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            storage: RwLock::new(storage),
            dm_scope: config.dm_scope,
            adapters: RwLock::new(HashMap::new()),
        }
    }

    /// Register an IM adapter.
    pub async fn register_adapter(&self, name: String, adapter: Arc<dyn IMAdapter>) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name, adapter);
    }

    /// Set the persistence backend.
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        *self.storage.write().await = Some(storage);
    }

    /// Compute session key from channel, message and optional account_id.
    fn compute_session_key(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> String {
        self.dm_scope
            .compute_session_key(channel, message, account_id)
    }

    /// Attempt to restore an archived session.
    /// Returns true iff restoration was attempted and succeeded.
    async fn try_restore_archived_session(&self, session_id: &str, channel: &str) -> bool {
        let storage = self.storage.read().await;
        let Some(storage) = storage.as_ref() else {
            return false;
        };

        let checkpoint = match storage.load_checkpoint(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) | Err(_) => return false,
        };

        if checkpoint.status != SessionStatus::Archived {
            return false;
        }

        // Send "正在恢复会话..." notification to the user
        let adapters = self.adapters.read().await;
        if let Some(adapter) = adapters.get(channel) {
            let notification = Message {
                id: format!("restore-{}", session_id),
                from: "system".to_string(),
                to: checkpoint
                    .chat_id
                    .as_deref()
                    .unwrap_or(session_id)
                    .to_string(),
                content: "正在恢复会话...".to_string(),
                channel: channel.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                metadata: std::collections::HashMap::new(),
            };
            if let Err(e) = adapter.send_message(&notification).await {
                warn!(session_id = %session_id, error = %e,
                    "failed to send restore notification");
            }
        }

        if let Err(e) = storage.restore_checkpoint(session_id).await {
            warn!(session_id = %session_id, error = %e,
                "failed to restore archived session");
            return false;
        }

        true
    }

    /// Find or create a session for the given channel and message.
    ///
    /// 1. Compute session_id from channel + message + account_id
    /// 2. If session exists in active table → return it
    /// 3. If not, try to restore from archived storage
    /// 4. If restoration succeeds → return restored session
    /// 5. Otherwise → create and register a new session
    ///
    /// Returns the session_id string on success.
    pub async fn find_or_create(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> Result<String, ProcessError> {
        let session_id = self.compute_session_key(channel, message, account_id);

        // Fast path: session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Ok(session_id);
            }
        }

        // Slow path: try archived session restoration
        let restored = self
            .try_restore_archived_session(&session_id, channel)
            .await;

        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            // Was restored by another task, or already existed
            return Ok(session_id);
        }

        if restored {
            // Reload checkpoint to obtain chat_id / agent_id for the new Session
            let storage = self.storage.read().await;
            if let Some(storage) = storage.as_ref() {
                if let Ok(Some(cp)) = storage.load_checkpoint(&session_id).await {
                    sessions.insert(
                        session_id.clone(),
                        Session {
                            id: session_id.clone(),
                            agent_id: cp.chat_id.unwrap_or_else(|| message.to.clone()),
                            channel: channel.to_string(),
                            created_at: chrono::Utc::now().timestamp(),
                        },
                    );
                }
            }
        } else {
            // No archived session — create a brand-new Session
            sessions.insert(
                session_id.clone(),
                Session {
                    id: session_id.clone(),
                    agent_id: message.to.clone(),
                    channel: channel.to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }

        Ok(session_id)
    }

    /// Get active sessions for an agent.
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.agent_id == agent_id)
            .cloned()
            .collect()
    }

    /// Check if a session with the given ID exists.
    pub async fn has_session(&self, session_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions.contains_key(session_id)
    }

    /// Get chat_id for a session.
    /// Returns the `agent_id` field of the session (which holds the chat_id per SessionManager convention).
    pub async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|s| s.agent_id.clone())
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{GatewayConfig, Message};
    use crate::session::persistence::{AgentRole, PersistenceError, SessionCheckpoint};
    use async_trait::async_trait;
    use tokio::sync::Mutex;

    fn test_config() -> GatewayConfig {
        GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: DmScope::PerChannelPeer,
        }
    }

    fn test_message() -> Message {
        Message {
            id: "msg-1".to_string(),
            from: "user-a".to_string(),
            to: "agent-b".to_string(),
            content: "hello".to_string(),
            channel: "feishu".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_find_or_create_existing_session() {
        let mgr = SessionManager::new(&test_config(), None);
        let msg = test_message();
        let session_id = mgr.compute_session_key("feishu", &msg, None);
        {
            let mut sessions = mgr.sessions.write().await;
            sessions.insert(
                session_id.clone(),
                Session {
                    id: session_id.clone(),
                    agent_id: "agent-b".to_string(),
                    channel: "feishu".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }

        // find_or_create should return the existing session (read-lock fast path)
        let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
        assert_eq!(result, session_id);
    }

    #[tokio::test]
    async fn test_find_or_create_new_user_creates_session() {
        let mgr = SessionManager::new(&test_config(), None);
        let msg = test_message();

        let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
        let sessions = mgr.sessions.read().await;
        assert!(sessions.contains_key(&result));
        let session = sessions.get(&result).unwrap();
        assert_eq!(session.agent_id, "agent-b");
        assert_eq!(session.channel, "feishu");
    }

    #[tokio::test]
    async fn test_archived_session_restoration() {
        use crate::session::persistence::SessionCheckpoint;
        use std::sync::Arc;

        // Build a mock storage that returns an Archived checkpoint
        let mock_storage = Arc::new(MockPersistService {
            archived_checkpoint: Mutex::new(Some(
                SessionCheckpoint::new("feishu:user-a:agent-b".to_string())
                    .with_status(SessionStatus::Archived)
                    .with_chat_id("agent-b".to_string()),
            )),
            restore_called: Mutex::new(false),
        });

        let config = test_config();
        let mut mgr = SessionManager::new(&config, Some(mock_storage.clone()));
        let msg = test_message();

        let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
        assert_eq!(result, "feishu:user-a:agent-b");

        // Verify restore was called
        let called = *mock_storage.restore_called.lock().await;
        assert!(called, "restore_checkpoint should have been called");
    }

    #[tokio::test]
    async fn test_find_or_create_no_storage() {
        // When storage is None, archived restoration returns false,
        // and find_or_create should still successfully create a new session.
        let mgr = SessionManager::new(&test_config(), None);
        let msg = test_message();

        let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
        assert_eq!(result, "feishu:user-a:agent-b");

        let sessions = mgr.sessions.read().await;
        assert!(sessions.contains_key(&result));
        let session = sessions.get(&result).unwrap();
        assert_eq!(session.agent_id, "agent-b");
        assert_eq!(session.channel, "feishu");
    }

    // Mock persistence service for tests
    struct MockPersistService {
        archived_checkpoint: Mutex<Option<crate::session::persistence::SessionCheckpoint>>,
        restore_called: Mutex<bool>,
    }

    #[async_trait::async_trait]
    impl PersistenceService for MockPersistService {
        async fn save_checkpoint(
            &self,
            _checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            Ok(())
        }

        async fn load_checkpoint(
            &self,
            _session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(self.archived_checkpoint.lock().await.take())
        }

        async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
            Ok(())
        }

        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn restore_checkpoint(
            &self,
            _session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            *self.restore_called.lock().await = true;
            Ok(self.archived_checkpoint.lock().await.take())
        }

        async fn archive_checkpoint(
            &self,
            _checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            Ok(())
        }

        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn purge_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
            Ok(())
        }

        async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
            Ok(())
        }

        async fn list_idle_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _idle_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn list_expired_archived_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _purge_after_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    // ── get_chat_id tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_session_manager_get_chat_id() {
        let mgr = SessionManager::new(&test_config(), None);
        let msg = test_message();
        let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();

        // get_chat_id should return the agent_id field (= chat_id)
        let chat_id = mgr.get_chat_id(&sid).await;
        assert!(chat_id.is_some(), "expected Some(chat_id), got None");
        assert_eq!(chat_id.unwrap(), "agent-b");
    }

    #[tokio::test]
    async fn test_session_manager_get_chat_id_missing() {
        let mgr = SessionManager::new(&test_config(), None);

        // Non-existent session_id → None
        let chat_id = mgr.get_chat_id("nonexistent-session-id").await;
        assert!(
            chat_id.is_none(),
            "expected None for missing session, got {chat_id:?}"
        );
    }
}
