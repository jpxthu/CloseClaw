//! Tests for the gateway `no_bootstrap` integration (issue #3475 Step 1.5).
//!
//! Covers the three state-establishment / rebuild scenarios:
//! 1. New session (resolve Path 3 creation)
//! 2. Archived restore (needs_conv = true, new ConversationSession)
//! 3. In-memory rebuild (`rebuild_archived_session_prompt`, needs_conv = false)
//!
//! An agent with `noBootstrap: true` must carry `no_bootstrap` into both the
//! `ConversationSession` cache and the rebuilt `InjectionParams`; default
//! agents (field `false` / absent) must stay unchanged.

use super::super::tests::test_config;
use super::super::SessionManager;
use crate::Message;
use closeclaw_common::injection_params::InjectionParams;
use closeclaw_common::{
    BootstrapMode, MemoryConfig, ModelSpec, PromptOverrides, ResolvedAgentConfig, SessionRole,
    SubagentsConfig, SystemPromptBuilder,
};
use closeclaw_config::manager::ConfigManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint, SessionStatus,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::RwLock;

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

/// Mock persistence: serves a cloned checkpoint on load/restore and an
/// optional archived-session ID for the routing-based archived lookup.
struct NbPersistMock {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    archived_id: Option<String>,
}

impl NbPersistMock {
    fn no_checkpoint() -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(None),
            archived_id: None,
        }
    }

    fn with_archived_checkpoint(cp: SessionCheckpoint, archived_id: Option<String>) -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(Some(cp)),
            archived_id,
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for NbPersistMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn load_archived_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(self.archived_id.clone())
    }
}

/// Builder that captures the `InjectionParams` received on rebuild, so tests
/// can assert the injection chain carries the `no_bootstrap` flag.
struct ParamsCaptureBuilder {
    captured: Arc<StdMutex<Option<InjectionParams>>>,
}

#[async_trait::async_trait]
impl SystemPromptBuilder for ParamsCaptureBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        String::new()
    }

    async fn build_prompt_with_params(&self, params: &InjectionParams) -> String {
        *self.captured.lock().unwrap() = Some(params.clone());
        String::new()
    }

    async fn invalidate_cache(&self) {}
}

/// ConfigManager pre-populated with `agent-b` whose `no_bootstrap` is set.
fn make_config_manager(no_bootstrap: bool) -> Arc<ConfigManager> {
    let config_dir = tempfile::TempDir::new().unwrap();
    let cm = Arc::new(ConfigManager::new(config_dir.path().to_path_buf()).unwrap());
    {
        let mut agents = cm.agents.write().unwrap();
        agents.insert(
            "agent-b".into(),
            ResolvedAgentConfig {
                id: "agent-b".into(),
                name: "agent-b".into(),
                parent_id: None,
                model: Some(ModelSpec::single("test-model")),
                workspace: None,
                agent_dir: None,
                bootstrap_mode: BootstrapMode::Full,
                skills: vec![],
                tools: vec![],
                disallowed_tools: vec![],
                subagents: SubagentsConfig::default(),
                memory: MemoryConfig::default(),
                hooks: vec![],
                parallel_tool_calls: true,
                memory_configured: false,
                no_bootstrap,
            },
        );
    }
    cm
}

/// SessionManager wired with the capturing prompt builder and the agent
/// config. Returns (mgr, capture) — caller keeps `capture` for assertions.
async fn setup_mgr(
    persist: Arc<dyn PersistenceService>,
    no_bootstrap: bool,
    captured: Arc<StdMutex<Option<InjectionParams>>>,
) -> SessionManager {
    let mgr = SessionManager::new(
        &test_config(),
        Some(persist),
        None,
        ReasoningLevel::default(),
    );
    mgr.set_system_prompt_builder(Arc::new(ParamsCaptureBuilder { captured }))
        .await;
    mgr.set_config_manager(make_config_manager(no_bootstrap))
        .await;
    mgr
}

fn archived_checkpoint(session_id: &str) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.to_string())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.platform = Some("feishu".to_string());
    cp
}

async fn assert_flag_carried(
    mgr: &SessionManager,
    session_id: &str,
    capture: &Arc<StdMutex<Option<InjectionParams>>>,
    expected: bool,
) {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let cs = cs.read().await;
    assert_eq!(
        cs.no_bootstrap(),
        expected,
        "ConversationSession cache flag mismatch"
    );
    let params = capture.lock().unwrap();
    let params = params
        .as_ref()
        .expect("rebuild must pass through build_prompt_with_params");
    assert_eq!(
        params.no_bootstrap, expected,
        "InjectionParams.no_bootstrap mismatch"
    );
}

// ── Scenario 1: new session (resolve Path 3 creation) ────────────────────

#[tokio::test]
async fn test_new_session_no_bootstrap_true_carries_into_injection() {
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::no_checkpoint()),
        true,
        capture.clone(),
    )
    .await;

    let msg = test_message();
    let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    assert_flag_carried(&mgr, &sid, &capture, true).await;
}

#[tokio::test]
async fn test_new_session_default_no_bootstrap_unchanged() {
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::no_checkpoint()),
        false,
        capture.clone(),
    )
    .await;

    let msg = test_message();
    let sid = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    assert_flag_carried(&mgr, &sid, &capture, false).await;
}

// ── Scenario 2: archived restore (needs_conv = true) ─────────────────────

#[tokio::test]
async fn test_archived_restore_no_bootstrap_true_carries_into_injection() {
    let sid = "nb-archived-restore".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::with_archived_checkpoint(
            archived_checkpoint(&sid),
            Some(sid.clone()),
        )),
        true,
        capture.clone(),
    )
    .await;

    // key_registry and conversation_sessions are empty → resolve falls
    // through to the archived check and creates a fresh ConversationSession.
    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    assert_flag_carried(&mgr, &sid, &capture, true).await;
}

/// E2-A control: an agent without `noBootstrap` (default `false`) restored
/// from the archive must keep loading bootstrap files — the flag carried
/// into cache and InjectionParams stays `false`.
#[tokio::test]
async fn test_archived_restore_default_no_bootstrap_unchanged() {
    let sid = "nb-archived-restore-default".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::with_archived_checkpoint(
            archived_checkpoint(&sid),
            Some(sid.clone()),
        )),
        false,
        capture.clone(),
    )
    .await;

    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    assert_flag_carried(&mgr, &sid, &capture, false).await;
}

// ── Scenario 3: in-memory rebuild (rebuild_archived_session_prompt) ──────

/// Marker workdir for the pre-populated session: the in-place rebuild must
/// keep it, whereas a fresh ConversationSession would get the fallback
/// workdir instead.
fn in_memory_workdir() -> PathBuf {
    std::env::temp_dir().join("closeclaw-nb-inmem-marker")
}

async fn pre_populate_in_memory_session(mgr: &SessionManager, session_id: &str) {
    let conv = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        in_memory_workdir(),
    )
    .with_system_prompt("stale-prompt");
    {
        let mut cs = mgr.conversation_sessions.write().await;
        cs.insert(session_id.to_string(), Arc::new(RwLock::new(conv)));
    }
}

#[tokio::test]
async fn test_in_memory_rebuild_no_bootstrap_true_carries_into_injection() {
    let sid = "nb-inmem-rebuild".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::with_archived_checkpoint(
            archived_checkpoint(&sid),
            Some(sid.clone()),
        )),
        true,
        capture.clone(),
    )
    .await;
    pre_populate_in_memory_session(&mgr, &sid).await;

    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    // In-place rebuild: the pre-populated workdir must survive (proves the
    // rebuild_archived_session_prompt path ran, not a fresh creation).
    let cs = mgr.get_conversation_session(&sid).await.unwrap();
    let cs = cs.read().await;
    assert_eq!(
        cs.workdir(),
        in_memory_workdir(),
        "session must be rebuilt in place, not re-created"
    );
    drop(cs);

    assert_flag_carried(&mgr, &sid, &capture, true).await;
}

#[tokio::test]
async fn test_in_memory_rebuild_default_no_bootstrap_unchanged() {
    let sid = "nb-inmem-rebuild-default".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mgr = setup_mgr(
        Arc::new(NbPersistMock::with_archived_checkpoint(
            archived_checkpoint(&sid),
            Some(sid.clone()),
        )),
        false,
        capture.clone(),
    )
    .await;
    pre_populate_in_memory_session(&mgr, &sid).await;

    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    assert_flag_carried(&mgr, &sid, &capture, false).await;
}
