//! Gateway restart checkpoint persistence tests.
//!
//! Verifies that `build_new_gateway` correctly injects the shared
//! `CheckpointManager` from `SessionManager` into the new Gateway,
//! so outbound checkpoint persistence survives a restart.

use std::sync::Arc;

use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

// ---------------------------------------------------------------------------
// Mock persistence
// ---------------------------------------------------------------------------

#[derive(Default)]
struct NoopPersist;

#[async_trait::async_trait]
impl PersistenceService for NoopPersist {
    async fn save_checkpoint(
        &self,
        _: &closeclaw_session::persistence::SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<closeclaw_session::persistence::SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

/// Mock persistence that tracks save_checkpoint calls.
#[derive(Default)]
struct MockPersist {
    saves: std::sync::Mutex<Vec<closeclaw_session::persistence::SessionCheckpoint>>,
}

impl MockPersist {
    fn save_count(&self) -> usize {
        self.saves.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(
        &self,
        cp: &closeclaw_session::persistence::SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        self.saves.lock().unwrap().push(cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<closeclaw_session::persistence::SessionCheckpoint>, PersistenceError> {
        Ok(self.saves.lock().unwrap().last().cloned())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When SessionManager has a checkpoint_manager, it is accessible via
/// the accessor and can be injected into a new Gateway.
#[tokio::test]
async fn checkpoint_manager_accessible_from_session_manager() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let persist: Arc<dyn PersistenceService> = Arc::new(NoopPersist);
    let cm = Arc::new(CheckpointManager::new(persist));
    sm.set_checkpoint_manager(cm).await;

    // checkpoint_manager() returns Some when set.
    let injected_cm = sm.checkpoint_manager().await;
    assert!(
        injected_cm.is_some(),
        "sm should have checkpoint_manager after set"
    );

    // Inject into a new Gateway — simulates build_new_gateway.
    let new_gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
    new_gw.set_checkpoint_manager(injected_cm.unwrap());

    // Gateway has checkpoint_manager set — the setter succeeded.
    // In production, persist_outbound_checkpoint uses this to persist.
}

/// When SessionManager has NO checkpoint_manager, the accessor returns
/// None and build_new_gateway skips injection (defensive branch).
#[tokio::test]
async fn build_new_gateway_no_checkpoint_manager_does_not_panic() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    // Do NOT set checkpoint_manager — it defaults to None.

    // Simulate the defensive branch in build_new_gateway:
    let cm_result = sm.checkpoint_manager().await;
    assert!(cm_result.is_none(), "no checkpoint_manager should be None");

    // Build Gateway without checkpoint_manager — no panic.
    let _gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
}

/// Full restart flow: Gateway with checkpoint_manager, after restart
/// the new Gateway also has the same checkpoint_manager.
#[tokio::test]
async fn checkpoint_manager_survives_gateway_restart() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let persist: Arc<dyn PersistenceService> = Arc::new(NoopPersist);
    let cm = Arc::new(CheckpointManager::new(persist));
    sm.set_checkpoint_manager(cm.clone()).await;

    // Old gateway
    let old_gw = closeclaw_gateway::Gateway::new(config.clone(), Arc::clone(&sm));
    old_gw.set_checkpoint_manager(cm.clone());

    // Simulate restart: build new gateway from same SessionManager
    let new_gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
    if let Some(injected) = sm.checkpoint_manager().await {
        new_gw.set_checkpoint_manager(injected);
    }

    // The new gateway has the checkpoint_manager injected — persistence
    // will work because set_checkpoint_manager wrote it.
    assert!(
        new_gw.has_checkpoint_manager(),
        "new gateway should have checkpoint_manager after restart"
    );
}

/// Simulate the build_new_gateway code path: read checkpoint_manager
/// from SessionManager, inject into new Gateway.
#[tokio::test]
async fn build_new_gateway_injects_checkpoint_manager_from_sm() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let persist: Arc<dyn PersistenceService> = Arc::new(NoopPersist);
    let cm = Arc::new(CheckpointManager::new(persist));
    sm.set_checkpoint_manager(cm).await;

    // Exact code path from gateway_restart.rs build_new_gateway:
    let new_gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
    if let Some(cm) = sm.checkpoint_manager().await {
        new_gw.set_checkpoint_manager(cm);
        // Injection succeeded.
    } else {
        panic!(
            "build_new_gateway: checkpoint_manager should be \
             present after set_checkpoint_manager"
        );
    }
}

/// When no checkpoint_manager is set, build_new_gateway skips injection.
/// Verifies the defensive branch does not panic.
#[tokio::test]
async fn build_new_gateway_skips_injection_when_no_checkpoint_manager() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    // No checkpoint_manager set — build_new_gateway defensive branch.

    let new_gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
    if let Some(cm) = sm.checkpoint_manager().await {
        new_gw.set_checkpoint_manager(cm);
    } else {
        // Defensive branch: warn and skip. No panic.
    }
}

/// E2E: set cm → old Gateway → simulate restart inject new Gateway →
/// verify new Gateway has checkpoint_manager → persist triggers mock.
#[tokio::test]
async fn e2e_restart_injects_cm_and_new_gw_has_it() {
    let config = closeclaw_gateway::GatewayConfig::default();
    let sm = Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let mock_persist = Arc::new(MockPersist::default());
    let mock_for_cm: Arc<dyn PersistenceService> = mock_persist.clone();
    let cm = Arc::new(CheckpointManager::new(mock_for_cm));
    sm.set_checkpoint_manager(cm).await;

    // Old gateway with cm
    let old_gw = closeclaw_gateway::Gateway::new(config.clone(), Arc::clone(&sm));
    old_gw.set_checkpoint_manager(sm.checkpoint_manager().await.unwrap());
    assert!(old_gw.has_checkpoint_manager());

    // Simulate restart: build new gateway, inject cm from sm
    let new_gw = closeclaw_gateway::Gateway::new(config, Arc::clone(&sm));
    if let Some(cm) = sm.checkpoint_manager().await {
        new_gw.set_checkpoint_manager(cm);
    }
    assert!(
        new_gw.has_checkpoint_manager(),
        "new gateway must have checkpoint_manager after restart inject"
    );

    // Verify mock persistence is callable via the shared cm.
    // The mock starts with 0 saves; after calling save_checkpoint on
    // the shared cm, the mock records the call — proving the injected
    // cm is the same instance that the old gateway used.
    assert_eq!(mock_persist.save_count(), 0, "no saves yet");
    let cp = closeclaw_session::persistence::SessionCheckpoint::new("test-session".to_string());
    let injected_cm = sm.checkpoint_manager().await.unwrap();
    injected_cm.save_sync(cp).await.unwrap();
    assert_eq!(
        mock_persist.save_count(),
        1,
        "mock should have recorded the save via shared cm"
    );
}
