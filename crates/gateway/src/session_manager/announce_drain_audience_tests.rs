//! Full-route audience tests: queue → drain → `route_system_notifications`
//! → `Gateway::send_system_notification`.
//!
//! Verifies the outbound-flow design doc「定向收件人」contract end to end
//! at the SessionManager routing level: the checkpoint `sender_id` is
//! resolved as the session's current user and handed to the Gateway
//! together with the audience declaration, so an OwnerOnly notification
//! reaches the plugin only for the Owner sender (silent drop otherwise).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::tests::clear_global_prompt_state;
use crate::{Gateway, GatewayConfig, SessionManager};
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_common::NotificationPriority;
use closeclaw_session::llm_session::NotificationAudience;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use tempfile::TempDir;

const ROUTE_SID: &str = "parent-audience-route";
const ROUTE_CHAT_ID: &str = "chat-audience-1";

/// Minimal in-memory mock persistence holding checkpoints with sender ids.
struct MockPersist(tokio::sync::Mutex<HashMap<String, SessionCheckpoint>>);

impl MockPersist {
    fn new() -> Self {
        Self(tokio::sync::Mutex::new(HashMap::new()))
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.0
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.0.lock().await.get(sid).cloned())
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

/// Spy plugin (platform "mock") recording every delivered text.
struct RecordingPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl IMPlugin for RecordingPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        if !text.is_empty() {
            self.messages.lock().expect("spy poisoned").push(text);
        }
        Ok(())
    }
}

/// Build SessionManager + Gateway wired to mock persistence with the
/// given checkpoint `sender_id`, a conversation session registered on
/// channel "mock" (chat id via the `agent_id` convention), and a
/// recording plugin registered under that channel.
async fn setup_route_test(
    sender_id: &str,
) -> (Arc<SessionManager>, Arc<Gateway>, Arc<Mutex<Vec<String>>>) {
    clear_global_prompt_state();
    let tmp = TempDir::new().unwrap();

    // Checkpoint storage carrying the current user — the route reads it
    // via SessionManager::get_sender_id (workflow_owner.rs 取法).
    let persist = Arc::new(MockPersist::new());
    let mut cp = SessionCheckpoint::new(ROUTE_SID.to_string());
    cp.sender_id = Some(sender_id.to_string());
    persist.save_checkpoint(&cp).await.unwrap();

    let mgr = Arc::new(SessionManager::new(
        &super::tests::test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        Some(tmp.path().to_path_buf()),
        Default::default(),
    ));

    // Register the session record: channel + chat id (agent_id slot).
    mgr.sessions.write().await.insert(
        ROUTE_SID.to_string(),
        crate::Session {
            id: ROUTE_SID.to_string(),
            agent_id: ROUTE_CHAT_ID.to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    // ConversationSession so the queue drain path finds the session.
    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            ROUTE_SID.to_string(),
            "test-model".to_string(),
            PathBuf::from(".").join("audience-route-test"),
        ),
    ));
    mgr.conversation_sessions
        .write()
        .await
        .insert(ROUTE_SID.to_string(), cs);

    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            persist.clone() as Arc<dyn PersistenceService>
        ),
    );

    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = Arc::new(
        Gateway::new_for_tests(test_gateway_config(), Arc::clone(&mgr)).with_checkpoint_manager(cm),
    );
    gw.register_plugin(Arc::new(RecordingPlugin {
        messages: Arc::clone(&messages),
    }) as Arc<dyn IMPlugin>)
        .await;
    (mgr, gw, messages)
}

fn test_gateway_config() -> GatewayConfig {
    GatewayConfig {
        name: "audience_route_test".into(),
        ..Default::default()
    }
}

/// OwnerOnly notification + checkpoint sender is the Owner → the
/// prefixed text is delivered to the session channel via the plugin.
#[tokio::test]
async fn test_route_delivers_owner_only_to_owner_sender() {
    let (mgr, gw, messages) = setup_route_test("owner").await;
    {
        let cs = mgr.get_conversation_session(ROUTE_SID).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification_with_audience(
            "cache anomaly notice".into(),
            NotificationPriority::Now,
            NotificationAudience::OwnerOnly,
        );
    }
    mgr.drain_and_inject_announces(ROUTE_SID, Some(&gw)).await;
    let delivered = messages.lock().unwrap().clone();
    assert_eq!(
        delivered.len(),
        1,
        "OwnerOnly notification must reach the Owner sender, got: {delivered:?}"
    );
    assert!(delivered[0].contains("cache anomaly notice"));
}

/// OwnerOnly notification + checkpoint sender is NOT the Owner →
/// silently dropped (plugin receives nothing, no error).
#[tokio::test]
async fn test_route_drops_owner_only_for_non_owner_sender() {
    let (mgr, gw, messages) = setup_route_test("user-a").await;
    {
        let cs = mgr.get_conversation_session(ROUTE_SID).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification_with_audience(
            "cache anomaly notice".into(),
            NotificationPriority::Now,
            NotificationAudience::OwnerOnly,
        );
    }
    mgr.drain_and_inject_announces(ROUTE_SID, Some(&gw)).await;
    let delivered = messages.lock().unwrap().clone();
    assert!(
        delivered.is_empty(),
        "OwnerOnly notification must be silently dropped for non-owner, got: {delivered:?}"
    );
    // The entry was consumed: the queue is empty (drop ≠ re-queue).
    assert!(mgr
        .get_conversation_session(ROUTE_SID)
        .await
        .unwrap()
        .read()
        .await
        .is_queue_empty());
}

/// `All` audience through the same route → delivered regardless of
/// sender role (legacy behavior unchanged).
#[tokio::test]
async fn test_route_delivers_all_audience_for_any_sender() {
    let (mgr, gw, messages) = setup_route_test("user-a").await;
    {
        let cs = mgr.get_conversation_session(ROUTE_SID).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_system_notification("plain queue notice".into(), NotificationPriority::Next);
    }
    mgr.drain_and_inject_announces(ROUTE_SID, Some(&gw)).await;
    let delivered = messages.lock().unwrap().clone();
    assert_eq!(
        delivered.len(),
        1,
        "All-audience notification must always be delivered, got: {delivered:?}"
    );
    assert!(delivered[0].contains("plain queue notice"));
}
