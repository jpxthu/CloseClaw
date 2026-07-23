//! Tests for `session_helpers::compute_session_workdir`.

use super::session_helpers::{compute_session_workdir, generate_session_id};
use crate::Message;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use std::path::PathBuf;
use std::sync::Arc;

// ── Mock persistence service ─────────────────────────────────────────────

struct MockPersist {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
}

impl MockPersist {
    fn new(checkpoint: Option<SessionCheckpoint>) -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(checkpoint),
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
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
        Ok(None)
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
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
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-fallback".to_string(),
        to: "agent-fallback".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    }
}

fn make_cm(checkpoint: Option<SessionCheckpoint>) -> CheckpointManager<dyn PersistenceService> {
    CheckpointManager::new(Arc::new(MockPersist::new(checkpoint)))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_restored_checkpoint_both_ids() {
    let cp = SessionCheckpoint::new("s1".to_string())
        .with_agent_id("cp-agent".to_string())
        .with_sender_id("cp-user".to_string());
    let cm = make_cm(Some(cp));
    let msg = test_message();
    let wd = Some(PathBuf::from("/tmp/test-ws"));

    let result = compute_session_workdir(true, "s1", &msg, &wd, &cm)
        .await
        .unwrap();

    // Should contain both checkpoint ids, not message fallbacks
    assert!(
        result.to_string_lossy().contains("cp-agent"),
        "agent_id should come from checkpoint: {:?}",
        result
    );
    assert!(
        result.to_string_lossy().contains("cp-user"),
        "user_id should come from checkpoint: {:?}",
        result
    );
    assert!(
        !result.to_string_lossy().contains("agent-fallback"),
        "should NOT use message.to: {:?}",
        result
    );
    assert!(
        !result.to_string_lossy().contains("user-fallback"),
        "should NOT use message.from: {:?}",
        result
    );
}

#[tokio::test]
async fn test_restored_sender_id_missing_fallback_to_message_from() {
    // Old checkpoint without sender_id
    let cp = SessionCheckpoint::new("s2".to_string()).with_agent_id("cp-agent".to_string());
    let cm = make_cm(Some(cp));
    let msg = test_message();
    let wd = Some(PathBuf::from("/tmp/test-ws"));

    let result = compute_session_workdir(true, "s2", &msg, &wd, &cm)
        .await
        .unwrap();

    assert!(
        result.to_string_lossy().contains("cp-agent"),
        "agent_id from checkpoint: {:?}",
        result
    );
    assert!(
        result.to_string_lossy().contains("user-fallback"),
        "sender_id missing → fallback to message.from: {:?}",
        result
    );
}

#[tokio::test]
async fn test_restored_agent_id_missing_fallback_to_message_to() {
    // Checkpoint without agent_id
    let cp = SessionCheckpoint::new("s3".to_string()).with_sender_id("cp-user".to_string());
    let cm = make_cm(Some(cp));
    let msg = test_message();
    let wd = Some(PathBuf::from("/tmp/test-ws"));

    let result = compute_session_workdir(true, "s3", &msg, &wd, &cm)
        .await
        .unwrap();

    assert!(
        result.to_string_lossy().contains("agent-fallback"),
        "agent_id missing → fallback to message.to: {:?}",
        result
    );
    assert!(
        result.to_string_lossy().contains("cp-user"),
        "sender_id from checkpoint: {:?}",
        result
    );
}

#[tokio::test]
async fn test_restored_workspace_dir_none_returns_tmp() {
    let cp = SessionCheckpoint::new("s4".to_string())
        .with_agent_id("cp-agent".to_string())
        .with_sender_id("cp-user".to_string());
    let cm = make_cm(Some(cp));
    let msg = test_message();

    let result = compute_session_workdir(true, "s4", &msg, &None, &cm)
        .await
        .unwrap();

    assert_eq!(result, PathBuf::from("/tmp"));
}

#[tokio::test]
async fn test_restored_no_checkpoint_fallback_to_message() {
    // Checkpoint not found
    let cm = make_cm(None);
    let msg = test_message();
    let wd = Some(PathBuf::from("/tmp/test-ws"));

    let result = compute_session_workdir(true, "s5", &msg, &wd, &cm)
        .await
        .unwrap();

    assert!(
        result.to_string_lossy().contains("agent-fallback"),
        "no checkpoint → agent_id from message.to: {:?}",
        result
    );
    assert!(
        result.to_string_lossy().contains("user-fallback"),
        "no checkpoint → user_id from message.from: {:?}",
        result
    );
}

// ===================================================================
// Step 1.4: generate_session_id format tests
// ===================================================================

#[test]
fn test_generate_session_id_format() {
    let id = generate_session_id("my-agent");
    let parts: Vec<&str> = id.split('_').collect();
    assert_eq!(
        parts.len(),
        3,
        "session_id should have 3 parts separated by '_': {:?}",
        id
    );
    assert_eq!(parts[0], "my-agent", "first part should be agent_id");
}

#[test]
fn test_generate_session_id_hex_is_8_lowercase() {
    let id = generate_session_id("agent");
    let hex_part = id.rsplit('_').next().unwrap();
    assert_eq!(
        hex_part.len(),
        8,
        "hex part should be 8 chars: {:?}",
        hex_part
    );
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hex part should be all hex digits: {:?}",
        hex_part
    );
    assert_eq!(
        hex_part,
        hex_part.to_lowercase(),
        "hex part should be lowercase: {:?}",
        hex_part
    );
}

#[test]
fn test_generate_session_id_timestamp_is_valid_unix_seconds() {
    let id = generate_session_id("agent");
    let parts: Vec<&str> = id.split('_').collect();
    let ts: i64 = parts[1]
        .parse()
        .expect("timestamp part should be a valid integer");
    // Should be after 2020-01-01 (1577836800) and before year 2100
    assert!(ts > 1_577_836_800, "timestamp should be after 2020: {}", ts);
    assert!(
        ts < 4_102_444_800,
        "timestamp should be before 2100: {}",
        ts
    );
}

#[test]
fn test_generate_session_id_unique() {
    let id1 = generate_session_id("agent");
    let id2 = generate_session_id("agent");
    assert_ne!(id1, id2, "two calls should produce different IDs");
}

#[test]
fn test_generate_session_id_preserves_agent_id() {
    let id = generate_session_id("eda");
    assert!(
        id.starts_with("eda_"),
        "session_id should start with agent_id: {:?}",
        id
    );

    let id2 = generate_session_id("my-long-agent-name");
    assert!(
        id2.starts_with("my-long-agent-name_"),
        "session_id should preserve full agent_id: {:?}",
        id2
    );
}

#[test]
fn test_generate_session_id_hex_parts_are_unique() {
    // Generate multiple IDs and verify hex parts differ
    let ids: Vec<String> = (0..10).map(|_| generate_session_id("a")).collect();
    let hex_parts: Vec<&str> = ids
        .iter()
        .map(|id| id.rsplit('_').next().unwrap())
        .collect();
    // All 10 hex parts should be distinct (collision probability is negligible)
    let unique_count = hex_parts
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(unique_count, 10, "all hex parts should be unique");
}
