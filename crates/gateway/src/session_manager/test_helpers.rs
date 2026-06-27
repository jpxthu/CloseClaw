//! Shared test helpers for `announce_tests`.
//!
//! Extracted to keep `announce_tests.rs` under the file-size limit.
//! Only used by `announce_tests.rs` for now, but placed at the
//! `session_manager` level so other test modules (e.g. future
//! `flush_tests` extensions) can reuse it without circular imports.

use super::spawn::{ChildSessionInfo, SpawnMode};
use super::SessionManager;
use chrono::Utc;
use closeclaw_common::agent_lookup::config::SubagentsConfig;
use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
use closeclaw_llm::session::{ChatSession, ConversationSession, SessionMessage};
use closeclaw_llm::types::{ContentBlock, UnifiedResponse, UnifiedUsage};
use closeclaw_session::bootstrap::BootstrapMode;
use std::path::PathBuf;

/// Build a `ResolvedAgentConfig` for tests. Identical to the one in
/// `spawn_tests` / `announce_tests` — kept local to avoid a
/// cross-test-module import path.
pub(super) fn test_resolved_config(id: &str, workspace: Option<PathBuf>) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: None,
        source: ConfigSource::Merged,
    }
}

/// Build a `UnifiedResponse` with the given content blocks and zero usage.
pub(super) fn make_response(blocks: Vec<ContentBlock>) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: blocks,
        usage: UnifiedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".to_string()),
    }
}

/// Append a single assistant message containing the given content blocks
/// to a child session's `ConversationSession`. Used to simulate
/// `append_response` after a child turn completes.
pub(super) async fn append_assistant_to_child(
    mgr: &SessionManager,
    child_id: &str,
    blocks: Vec<ContentBlock>,
) {
    let cs = mgr
        .get_conversation_session(child_id)
        .await
        .expect("child ConversationSession should exist");
    let mut cs = cs.write().await;
    cs.append_response(make_response(blocks));
}

/// Register a parent session along with a `ConversationSession` so that
/// `push_announce`/`drain_announces` (which look up the parent via
/// `get_conversation_session`) succeed. Returns the parent session id.
///
/// This mirrors the real flow in `find_or_create` but is direct, so
/// tests don't depend on message routing logic.
pub(super) async fn setup_parent_with_conv(mgr: &SessionManager, parent_id: &str) -> String {
    use crate::Session;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "feishu".to_string(),
            created_at: Utc::now().timestamp(),
            depth: 0,
        },
    );
    let cs = Arc::new(RwLock::new(ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    )));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), cs);
    parent_id.to_string()
}

/// Drain the parent's announce queue, inject each event as a
/// `role="system"` `SessionMessage` using the documented
/// `[子 agent X] 任务已完成：\n<text>` format, and return the resulting
/// message history. Used by the system-injection test.
pub(super) async fn inject_events_and_return_messages(
    mgr: &SessionManager,
    parent_id: &str,
) -> Vec<SessionMessage> {
    let drained = mgr.drain_announces(parent_id).await;
    {
        let cs = mgr
            .get_conversation_session(parent_id)
            .await
            .expect("parent ConversationSession should exist");
        let mut cs = cs.write().await;
        for ev in &drained {
            cs.inject_system_message(format!(
                "[子 agent {}] 任务已完成：\n{}",
                ev.child_agent_id, ev.result_text
            ));
        }
    }
    let cs = mgr
        .get_conversation_session(parent_id)
        .await
        .expect("parent ConversationSession should exist");
    let messages = cs.read().await.messages().to_vec();
    messages
}

/// Register a child session under the given parent in the children
/// tracking table only (no `ConversationSession` is created). Used by
/// tests that don't need the child to have a real session — the
/// `try_push_announce` code path will simply skip the lookup if the
/// child has no `ConversationSession`.
pub(super) async fn register_child_only(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    mode: SpawnMode,
) {
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: agent_id.to_string(),
            depth: 1,
            mode,
        },
    )
    .await;
}

/// Create `N` run-mode children under the given parent, each with a
/// unique `worker-{i}` agent id and a unique `answer-{i}` assistant
/// message. Returns the list of generated child session ids.
pub(super) async fn spawn_n_run_children(
    mgr: &SessionManager,
    parent_id: &str,
    n: usize,
) -> Vec<String> {
    let mut child_ids: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let config = test_resolved_config(&format!("worker-{}", i), None);
        let child_id = mgr
            .create_child_session(
                &config,
                parent_id,
                1,
                &format!("task {}", i),
                true,
                None,
                SpawnMode::Run,
                false,
                None,
                None,
                None,
                3, // max_spawn_depth
            )
            .await
            .expect("create_child_session should succeed");

        append_assistant_to_child(
            mgr,
            &child_id,
            vec![ContentBlock::Text(format!("answer-{}", i))],
        )
        .await;

        child_ids.push(child_id);
    }
    child_ids
}

// ── Mock persistence service ──────────────────────────────────────────────

use closeclaw_session::persistence::{
    AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
};
pub struct MockPersistService {
    pub archived_checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    pub restore_called: tokio::sync::Mutex<bool>,
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistService {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.archived_checkpoint.lock().await.take())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().await = true;
        Ok(self.archived_checkpoint.lock().await.take())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}
