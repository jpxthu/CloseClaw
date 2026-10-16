//! Tests for the three remaining *new* ConversationSession paths that
//! establish runtime state (issue #3475 Step 1.8).
//!
//! Step 1.5 wired `no_bootstrap` into resolve.rs creation, archived restore
//! creation, and in-memory rebuild. Three further paths create a fresh
//! ConversationSession but only cached `bootstrap_mode`:
//! 1. `restore_helpers::rebuild_and_finalize_session` — checkpoint restore
//!    with `needs_conv = true` (resolve Path 2)
//! 2. `channel::force_new_for_channel` — `/new` command
//! 3. `recovery_injection::build_conv_session` — startup recovery injection
//!
//! Each scenario is covered in both states: `noBootstrap: true` must carry
//! into the `ConversationSession` cache and the rebuilt `InjectionParams`;
//! the default (`false`) must stay unchanged. Reuses the
//! `ParamsCaptureBuilder` pattern from `archived_rebuild/tests.rs`.

use super::tests::test_config;
use super::SessionManager;
use crate::Message;
use closeclaw_common::injection_params::InjectionParams;
use closeclaw_common::{
    BootstrapMode, MemoryConfig, ModelSpec, PromptOverrides, ResolvedAgentConfig, SessionRole,
    SubagentsConfig, SystemPromptBuilder,
};
use closeclaw_config::manager::ConfigManager;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint, SessionStatus,
};
use std::sync::{Arc, Mutex as StdMutex};

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

/// Clone-on-read persistence mock: serves the stored checkpoint from every
/// load/restore call so the resolve-restore, checkpoint-manager, and
/// workdir-computation reads all succeed against one stored checkpoint.
struct NbPathPersistMock {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
}

impl NbPathPersistMock {
    fn empty() -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(None),
        }
    }

    fn with_checkpoint(cp: SessionCheckpoint) -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(Some(cp)),
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for NbPathPersistMock {
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
        Ok(None)
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
/// Returns the backing TempDir so the caller keeps the config dir alive for
/// the whole test (no dangling path).
fn make_config_manager(no_bootstrap: bool) -> (Arc<ConfigManager>, tempfile::TempDir) {
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
    (cm, config_dir)
}

/// SessionManager wired with the capturing prompt builder and the agent
/// config. Returns (mgr, config_dir) — the caller keeps `config_dir` alive.
async fn setup_mgr(
    persist: Arc<dyn PersistenceService>,
    no_bootstrap: bool,
    captured: Arc<StdMutex<Option<InjectionParams>>>,
) -> (SessionManager, tempfile::TempDir) {
    let (config_manager, config_dir) = make_config_manager(no_bootstrap);
    let mgr = SessionManager::new(
        &test_config(),
        Some(persist),
        None,
        ReasoningLevel::default(),
    );
    mgr.set_system_prompt_builder(Arc::new(ParamsCaptureBuilder { captured }))
        .await;
    mgr.set_config_manager(config_manager).await;
    (mgr, config_dir)
}

/// Assert the flag landed in both the ConversationSession cache and the
/// captured InjectionParams.
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

// ── Scenario 1: checkpoint restore with needs_conv = true ────────────────

fn archived_checkpoint(session_id: &str) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.to_string())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.platform = Some("feishu".to_string());
    cp.sender_id = Some("user-a".to_string());
    cp
}

/// Pre-register the routing key so resolve takes Path 2 (registry hit +
/// no active session → archived restore creating a fresh ConversationSession
/// via `rebuild_and_finalize_session`).
async fn pre_register_routing_key(mgr: &SessionManager, session_id: &str) -> Message {
    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);
    mgr.key_registry
        .write()
        .await
        .insert(routing_key, session_id.to_string());
    msg
}

#[tokio::test]
async fn test_checkpoint_restore_new_conv_no_bootstrap_true() {
    let sid = "nb-restore-new-conv".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) = setup_mgr(
        Arc::new(NbPathPersistMock::with_checkpoint(archived_checkpoint(
            &sid,
        ))),
        true,
        capture.clone(),
    )
    .await;
    let msg = pre_register_routing_key(&mgr, &sid).await;

    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    assert_flag_carried(&mgr, &sid, &capture, true).await;
}

#[tokio::test]
async fn test_checkpoint_restore_new_conv_default_no_bootstrap_unchanged() {
    let sid = "nb-restore-new-conv-default".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) = setup_mgr(
        Arc::new(NbPathPersistMock::with_checkpoint(archived_checkpoint(
            &sid,
        ))),
        false,
        capture.clone(),
    )
    .await;
    let msg = pre_register_routing_key(&mgr, &sid).await;

    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, sid);

    assert_flag_carried(&mgr, &sid, &capture, false).await;
}

// ── Scenario 2: /new command (force_new_for_channel) ─────────────────────

#[tokio::test]
async fn test_force_new_for_channel_no_bootstrap_true() {
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) =
        setup_mgr(Arc::new(NbPathPersistMock::empty()), true, capture.clone()).await;

    let sid = mgr.force_new_for_channel("feishu", "agent-b").await;

    assert_flag_carried(&mgr, &sid, &capture, true).await;
}

#[tokio::test]
async fn test_force_new_for_channel_default_no_bootstrap_unchanged() {
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) =
        setup_mgr(Arc::new(NbPathPersistMock::empty()), false, capture.clone()).await;

    let sid = mgr.force_new_for_channel("feishu", "agent-b").await;

    assert_flag_carried(&mgr, &sid, &capture, false).await;
}

// ── Scenario 3: startup recovery injection (build_conv_session) ──────────

fn recovery_checkpoint(session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint::new(session_id.to_string())
        .with_status(SessionStatus::Active)
        .with_agent_id("agent-b".to_string())
        .with_recovery_notification(Some("recovery notice".to_string()))
}

/// `build_conv_session` rebuilds before the prompt builder is wired (the
/// builder arrives with `wire_conv_dependencies` afterwards), so capture the
/// carried flag via a follow-up rebuild on the recovered session.
async fn assert_recovery_flag_carried(
    mgr: &SessionManager,
    session_id: &str,
    capture: &Arc<StdMutex<Option<InjectionParams>>>,
    expected: bool,
) {
    let cs = mgr
        .get_conversation_session(session_id)
        .await
        .expect("recovery must create a ConversationSession");
    assert_eq!(
        cs.read().await.no_bootstrap(),
        expected,
        "ConversationSession cache flag mismatch"
    );

    cs.write()
        .await
        .rebuild_system_prompt(session_id, "agent-b", None)
        .await;
    let params = capture.lock().unwrap();
    let params = params
        .as_ref()
        .expect("rebuild must pass through build_prompt_with_params");
    assert_eq!(
        params.no_bootstrap, expected,
        "InjectionParams.no_bootstrap mismatch"
    );
}

#[tokio::test]
async fn test_startup_recovery_new_conv_no_bootstrap_true() {
    let sid = "nb-recovery-new-conv".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) = setup_mgr(
        Arc::new(NbPathPersistMock::with_checkpoint(recovery_checkpoint(
            &sid,
        ))),
        true,
        capture.clone(),
    )
    .await;

    mgr.inject_startup_recovery_notifications(std::slice::from_ref(&sid))
        .await;

    assert_recovery_flag_carried(&mgr, &sid, &capture, true).await;
}

#[tokio::test]
async fn test_startup_recovery_new_conv_default_no_bootstrap_unchanged() {
    let sid = "nb-recovery-new-conv-default".to_string();
    let capture = Arc::new(StdMutex::new(None::<InjectionParams>));
    let (mgr, _config_dir) = setup_mgr(
        Arc::new(NbPathPersistMock::with_checkpoint(recovery_checkpoint(
            &sid,
        ))),
        false,
        capture.clone(),
    )
    .await;

    mgr.inject_startup_recovery_notifications(std::slice::from_ref(&sid))
        .await;

    assert_recovery_flag_carried(&mgr, &sid, &capture, false).await;
}
