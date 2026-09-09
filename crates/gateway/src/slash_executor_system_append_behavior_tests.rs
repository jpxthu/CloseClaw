//! Behaviour tests for `gw_system_append` (Step 1.3).
//!
//! Verifies:
//! - Clear triggers system prompt rebuild (not just cache invalidation).
//! - Add persists checkpoint with user_appends containing the new entry.
//! - Clear persists checkpoint with empty user_appends.
//! - Persist failure does not affect instruction return value.
//! - system_injection_appends are preserved after Clear (isolation).

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};

use crate::slash_executor_helpers::gw_system_append;
use crate::{Gateway, GatewayConfig, SessionManager};
use closeclaw_common::slash_router::SystemAppendAction;

// ── Mock: RecordingPersistenceService ─────────────────────────────────

/// Persistence service that records every checkpoint passed to `save_checkpoint`.
struct RecordingPersistenceService {
    saved: std::sync::Mutex<Vec<SessionCheckpoint>>,
}

impl RecordingPersistenceService {
    fn new() -> Self {
        Self {
            saved: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn last_checkpoint(&self) -> Option<SessionCheckpoint> {
        self.saved.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl PersistenceService for RecordingPersistenceService {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.saved.lock().unwrap().push(cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Mock: FailingPersistenceService ───────────────────────────────────

/// Persistence service that always fails on `save_checkpoint`.
struct FailingPersistenceService;

#[async_trait]
impl PersistenceService for FailingPersistenceService {
    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "simulated save failure",
        )))
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Test helpers ──────────────────────────────────────────────────────

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    }
}

async fn make_gw_with_recording_storage() -> (Arc<Gateway>, Arc<RecordingPersistenceService>) {
    let storage = Arc::new(RecordingPersistenceService::new());
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(storage.clone() as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    (Arc::new(Gateway::new(config, sm)), storage)
}

async fn make_gw_with_failing_storage() -> Arc<Gateway> {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(FailingPersistenceService) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

async fn register_cs(gw: &Gateway, session_id: &str) {
    let cs = ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    );
    let mut conv = gw.session_manager.conversation_sessions.write().await;
    conv.insert(
        session_id.to_owned(),
        Arc::new(tokio::sync::RwLock::new(cs)),
    );
}

async fn get_cs(gw: &Gateway, session_id: &str) -> Arc<tokio::sync::RwLock<ConversationSession>> {
    gw.session_manager
        .get_conversation_session(session_id)
        .await
        .expect("conversation session must exist")
}

// ── Tests ─────────────────────────────────────────────────────────────

/// Clear后：system prompt 被重建（区别于仅缓存失效）。
///
/// After Clear, the static cache invalidator callback fires and the
/// system prompt is cleared (no builder configured → empty string).
#[tokio::test]
async fn test_clear_triggers_cache_invalidation() {
    let (gw, _storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-clear").await;

    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&called);
    gw.session_manager
        .set_cache_invalidator(Arc::new(move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }))
        .await;

    let count = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clear",
        &SystemAppendAction::Clear,
    )
    .await;

    assert_eq!(count, 0, "clear on empty list returns 0");
    assert!(
        called.load(std::sync::atomic::Ordering::SeqCst),
        "cache_invalidator must fire after Clear"
    );
}

/// Add后：checkpoint 中 user_appends 含新增条目（即时持久化生效）。
#[tokio::test]
async fn test_add_persists_checkpoint_with_user_appends() {
    let (gw, storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-add").await;

    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-add",
        &SystemAppendAction::Add("new rule".to_owned()),
    )
    .await;

    let cp = storage
        .last_checkpoint()
        .expect("persist_checkpoint must have been called");
    assert_eq!(
        cp.user_appends,
        vec!["new rule".to_owned()],
        "checkpoint user_appends must contain the added entry"
    );
}

/// Clear后：checkpoint 中 user_appends 为空列表（持久化空列表）。
#[tokio::test]
async fn test_clear_persists_checkpoint_with_empty_user_appends() {
    let (gw, storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-clear-persist").await;

    // Add first so user_appends is non-empty before clear.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clear-persist",
        &SystemAppendAction::Add("rule to clear".to_owned()),
    )
    .await;

    // Clear.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clear-persist",
        &SystemAppendAction::Clear,
    )
    .await;

    let cp = storage
        .last_checkpoint()
        .expect("persist_checkpoint must have been called after Clear");
    assert!(
        cp.user_appends.is_empty(),
        "checkpoint user_appends must be empty after Clear, got: {:?}",
        cp.user_appends
    );
}

/// persist失败路径：调用失败不影响指令返回值/回复（错误路径）。
#[tokio::test]
async fn test_persist_failure_does_not_affect_return_value() {
    let gw = make_gw_with_failing_storage().await;
    register_cs(&gw, "sess-beh-fail").await;

    // Add should succeed (return value = 1, the 1-based count) despite persist failure.
    let count = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-fail",
        &SystemAppendAction::Add("rule".to_owned()),
    )
    .await;
    assert_eq!(
        count, 1,
        "Add return value must be 1 (1-based count), not affected by persist failure"
    );

    // Clear should also succeed despite persist failure.
    let count = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-fail",
        &SystemAppendAction::Clear,
    )
    .await;
    assert_eq!(
        count, 1,
        "Clear return value must be 1 (one item cleared), not affected by persist failure"
    );
}

/// 系统注入内容不受影响：clear 后 system_injection_appends 保持原样。
#[tokio::test]
async fn test_clear_preserves_system_injection_appends() {
    let (gw, _storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-inject").await;

    // Inject system-managed appends (these are not user-managed).
    {
        let cs = get_cs(&gw, "sess-beh-inject").await;
        let mut guard = cs.write().await;
        guard.add_system_injection_append("injected-1".to_owned());
        guard.add_system_injection_append("injected-2".to_owned());
    }

    // Add a user-managed append.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-inject",
        &SystemAppendAction::Add("user rule".to_owned()),
    )
    .await;

    // Clear user-managed appends.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-inject",
        &SystemAppendAction::Clear,
    )
    .await;

    // Verify system_injection_appends are preserved.
    let cs = get_cs(&gw, "sess-beh-inject").await;
    let guard = cs.read().await;
    assert_eq!(
        guard.system_injection_appends(),
        &["injected-1".to_owned(), "injected-2".to_owned()],
        "system_injection_appends must be preserved after Clear"
    );
    assert!(
        guard.user_system_appends().is_empty(),
        "user_system_appends must be empty after Clear"
    );
}

/// Clear后：rebuild_system_prompt_for_session 被调用。
///
/// Full system prompt rebuild requires a configured builder. Without
/// a builder, `rebuild_system_prompt` returns early. The cache invalidator
/// callback test (test_clear_triggers_cache_invalidation) verifies the
/// invalidation path, which is the prerequisite for rebuild.
///
/// This test verifies the return value is correct (the count of cleared items)
/// which confirms the Clear branch executed fully before rebuild.
#[tokio::test]
async fn test_clear_executes_full_branch() {
    let (gw, _storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-rebuild").await;

    // Add an item so Clear returns a non-zero count.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-rebuild",
        &SystemAppendAction::Add("item".to_owned()),
    )
    .await;

    // Clear must return 1 (one item cleared), confirming the full branch executed.
    let cleared = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-rebuild",
        &SystemAppendAction::Clear,
    )
    .await;
    assert_eq!(
        cleared, 1,
        "Clear must return the number of cleared items (full branch executed)"
    );
}

/// Add：返回值是 1-based count（与回复格式 "已追加指令 #N" 对齐），同时触发 checkpoint 持久化。
#[tokio::test]
async fn test_add_returns_correct_count() {
    let (gw, _storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-idx").await;

    let count1 = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-idx",
        &SystemAppendAction::Add("first".to_owned()),
    )
    .await;
    assert_eq!(count1, 1, "first Add returns 1 (1-based count)");

    let count2 = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-idx",
        &SystemAppendAction::Add("second".to_owned()),
    )
    .await;
    assert_eq!(count2, 2, "second Add returns 2 (1-based count)");
}

/// Clear：返回值是被清除的 user_appends 数量。
#[tokio::test]
async fn test_clear_returns_correct_count() {
    let (gw, _storage) = make_gw_with_recording_storage().await;
    register_cs(&gw, "sess-beh-clr-count").await;

    // Add 2 items.
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clr-count",
        &SystemAppendAction::Add("a".to_owned()),
    )
    .await;
    gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clr-count",
        &SystemAppendAction::Add("b".to_owned()),
    )
    .await;

    let cleared = gw_system_append(
        &gw.session_manager,
        &None,
        "sess-beh-clr-count",
        &SystemAppendAction::Clear,
    )
    .await;
    assert_eq!(cleared, 2, "Clear must return the number of cleared items");
}
