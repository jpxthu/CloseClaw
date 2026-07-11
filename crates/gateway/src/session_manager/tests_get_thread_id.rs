use super::test_helpers::MockPersistService;
use super::tests::{make_test_mgr, test_config};
use crate::session_manager::SessionManager;
use closeclaw_session::persistence::{ReasoningLevel, SessionCheckpoint};
use std::sync::Arc;

// ── get_thread_id tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_thread_id_no_storage() {
    let mgr = make_test_mgr(None);
    let result = mgr.get_thread_id("sess_no_storage").await;
    assert!(
        result.is_none(),
        "should return None when no storage is configured"
    );
}

#[tokio::test]
async fn test_get_thread_id_returns_none_when_no_thread_id() {
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: tokio::sync::Mutex::new(Some(SessionCheckpoint::new(
            "sess_no_tid".to_string(),
        ))),
        restore_called: tokio::sync::Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock_storage),
        None,
        ReasoningLevel::default(),
    );
    let result = mgr.get_thread_id("sess_no_tid").await;
    assert!(
        result.is_none(),
        "should return None when checkpoint has no thread_id"
    );
}

#[tokio::test]
async fn test_get_thread_id_returns_value_when_set() {
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: tokio::sync::Mutex::new(Some(
            SessionCheckpoint::new("sess_has_tid".to_string())
                .with_thread_id("omt_from_checkpoint".into()),
        )),
        restore_called: tokio::sync::Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock_storage),
        None,
        ReasoningLevel::default(),
    );
    let result = mgr.get_thread_id("sess_has_tid").await;
    assert_eq!(result.as_deref(), Some("omt_from_checkpoint"));
}
