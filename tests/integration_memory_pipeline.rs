//! Integration tests for the Memory pipeline.
//!
//! Verifies the full data flow: transcript → mining → SQLite → dreaming →
//! MEMORY.md, including edge cases.

use std::sync::Arc;

use async_trait::async_trait;
use tempfile::TempDir;

use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_memory::miner::{MinerConfig, MiningEntity, MiningEvent, MiningEventCategory};
use closeclaw_memory::miner_llm::{MinerLlmCaller, MinerLlmError};
use closeclaw_session::persistence::{
    DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint,
};

// ── Test helpers ─────────────────────────────────────────────────────────

/// In-memory persistence service for integration tests.
#[derive(Debug, Default)]
struct TestPersistence {
    checkpoints: std::sync::Mutex<Vec<SessionCheckpoint>>,
    archived: std::sync::Mutex<Vec<SessionCheckpoint>>,
    mined_ids: std::sync::Mutex<Vec<String>>,
    dreaming_statuses: std::sync::Mutex<Vec<(String, DreamingStatus)>>,
}

impl TestPersistence {
    fn add_active(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
    }

    fn add_archived(&self, cp: SessionCheckpoint) {
        self.archived.lock().unwrap().push(cp);
    }

    #[allow(dead_code)]
    fn mined_ids(&self) -> Vec<String> {
        self.mined_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl PersistenceService for TestPersistence {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints.lock().unwrap().push(cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .find(|cp| cp.session_id == sid)
            .cloned())
    }

    async fn load_archived_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .archived
            .lock()
            .unwrap()
            .iter()
            .find(|cp| cp.session_id == sid)
            .cloned())
    }

    async fn delete_checkpoint(&self, sid: &str) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .unwrap()
            .retain(|cp| cp.session_id != sid);
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .archived
            .lock()
            .unwrap()
            .iter()
            .filter(|cp| !cp.mined)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .filter(|cp| cp.mined && cp.dreaming_status != DreamingStatus::Completed)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn mark_mined(&self, sid: &str) -> Result<(), PersistenceError> {
        self.mined_ids.lock().unwrap().push(sid.into());
        Ok(())
    }

    async fn update_dreaming_status(
        &self,
        sid: &str,
        status: DreamingStatus,
    ) -> Result<(), PersistenceError> {
        self.dreaming_statuses
            .lock()
            .unwrap()
            .push((sid.into(), status));
        Ok(())
    }
}

/// Mock LLM caller that returns canned events and entities.
struct MockMinerLlm {
    events: Vec<MiningEvent>,
    entities: Vec<Vec<MiningEntity>>,
    fail_extract: bool,
    fail_assign: bool,
}

impl MockMinerLlm {
    fn single_error_event() -> Self {
        Self {
            events: vec![MiningEvent {
                title: "Wrong deployment".into(),
                summary: "Deployed to prod without testing".into(),
                body: "Owner asked to deploy. Agent deployed without running tests first.".into(),
                category: MiningEventCategory::Error,
                lesson: Some("Always run tests before deploying".into()),
            }],
            entities: vec![vec![MiningEntity {
                entity_type: "action".into(),
                name: "wrong deployment".into(),
                description: "deployed without testing".into(),
            }]],
            fail_extract: false,
            fail_assign: false,
        }
    }

    fn empty() -> Self {
        Self {
            events: Vec::new(),
            entities: Vec::new(),
            fail_extract: false,
            fail_assign: false,
        }
    }

    fn failing() -> Self {
        Self {
            events: Vec::new(),
            entities: Vec::new(),
            fail_extract: true,
            fail_assign: true,
        }
    }
}

#[async_trait]
impl MinerLlmCaller for MockMinerLlm {
    async fn extract_events(
        &self,
        _transcript: &str,
        _existing_events: &str,
        _existing_memory: &str,
    ) -> Result<Vec<MiningEvent>, MinerLlmError> {
        if self.fail_extract {
            return Err(MinerLlmError::Llm("mock failure".into()));
        }
        Ok(self.events.clone())
    }

    async fn assign_entities(
        &self,
        events: &[MiningEvent],
        _catalog: &str,
    ) -> Result<Vec<Vec<MiningEntity>>, MinerLlmError> {
        if self.fail_assign {
            return Err(MinerLlmError::Llm("mock failure".into()));
        }
        if self.entities.is_empty() {
            return Ok(events.iter().map(|_| Vec::new()).collect());
        }
        Ok(self.entities.clone())
    }
}

/// Create a temp dir, SqliteStorage-backed miner, and dreaming pipeline
/// configured for integration testing.
fn setup(miner_llm: Box<dyn MinerLlmCaller>) -> (TempDir, MemoryMiner, DreamingPipeline) {
    setup_with_config(miner_llm, MinerConfig::default())
}

/// Like [`setup`] but accepts a custom [`MinerConfig`].
fn setup_with_config(
    miner_llm: Box<dyn MinerLlmCaller>,
    miner_config: MinerConfig,
) -> (TempDir, MemoryMiner, DreamingPipeline) {
    let tmp = tempfile::TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let db_path = data_dir.join("memory/memory.db");
    let memory_md_path = data_dir.join("memory/MEMORY.md");

    // Ensure parent dirs exist.
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let miner = MemoryMiner::new(
        miner_config,
        miner_llm,
        &db_path,
        memory_md_path.to_str().unwrap(),
    );

    let dreaming_config = closeclaw_config::agents::DreamingConfig {
        enabled: Some(true),
        ..Default::default()
    };
    let dreaming = DreamingPipeline::with_config(dreaming_config)
        .with_db_path(&db_path)
        .with_memory_md_path(memory_md_path.to_str().unwrap());

    (tmp, miner, dreaming)
}

/// MinerConfig with permissive thresholds for short test transcripts.
fn test_miner_config() -> MinerConfig {
    MinerConfig {
        enabled: true,
        clean_rules: closeclaw_config::agents::TranscriptCleanRules {
            min_turns: Some(1),
            min_owner_msgs: Some(1),
            format: Some("md".to_string()),
        },
        ..MinerConfig::default()
    }
}

/// Create an archived, unmined checkpoint with pending messages
/// formatted as a transcript.
fn make_archived_checkpoint(
    session_id: &str,
    transcript_lines: &[(&str, &str)],
) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.mined = false;
    cp.dreaming_status = DreamingStatus::Pending;
    cp.agent_id = Some("test-agent".into());

    // Populate outbound_pending with transcript lines.
    // PendingMessage.message_id stores the role.
    cp.outbound_pending = transcript_lines
        .iter()
        .map(|(role, content)| {
            closeclaw_session::persistence::PendingMessage::new(
                role.to_string(),
                content.to_string(),
            )
        })
        .collect();

    cp
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Full pipeline: transcript → mining → SQLite → dreaming (enabled) → MEMORY.md.
#[tokio::test]
async fn test_full_pipeline_transcript_to_memory_md() {
    let (tmp, miner, dreaming) = setup_with_config(
        Box::new(MockMinerLlm::single_error_event()),
        test_miner_config(),
    );
    let storage = Arc::new(TestPersistence::default());

    let transcript = vec![
        ("user", "Please deploy the app"),
        ("assistant", "Deploying now"),
        ("user", "Wait, did you run tests?"),
        ("assistant", "No, I forgot"),
        ("user", "Always run tests before deploying!"),
        ("assistant", "Understood, I will run tests first"),
        ("user", "Good, let me know when done"),
        ("assistant", "Tests passed, deploying"),
    ];
    let cp = make_archived_checkpoint("sess-1", &transcript);
    storage.add_archived(cp);

    // Also add as active so load_checkpoint works for mine_session.
    storage.add_active(make_archived_checkpoint("sess-1", &transcript));

    // Step 1: Mine the session.
    let archived = storage
        .load_archived_checkpoint("sess-1")
        .await
        .unwrap()
        .unwrap();
    let raw_transcript = format_transcript(&archived.outbound_pending);
    let result = miner
        .mine_session_from_checkpoint(
            "sess-1",
            &raw_transcript,
            "test-agent",
            &archived,
            storage.as_ref(),
        )
        .await
        .unwrap();

    assert_eq!(result.events.len(), 1, "should extract 1 event");
    assert_eq!(result.events[0].category, MiningEventCategory::Error);
    assert_eq!(result.events[0].title, "Wrong deployment");

    // Step 2: Verify SQLite was populated.
    let db_path = tmp.path().join("memory/memory.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row: &rusqlite::Row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1, "should have 1 event in SQLite");

    // Step 3: Verify entity was written.
    let entity_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();
    assert!(entity_count >= 1, "should have at least 1 entity");

    // Step 4: Mark mined and run dreaming.
    // Move checkpoint to "mined" state so dreaming can process it.
    storage.add_active({
        let mut cp = make_archived_checkpoint("sess-1", &transcript);
        cp.mined = true;
        cp.dreaming_status = DreamingStatus::Pending;
        cp
    });

    let result = dreaming.run_once(storage.as_ref()).await;
    assert!(result.is_ok(), "dreaming should succeed: {result:?}");

    // Step 5: Verify MEMORY.md was written.
    let memory_md_path = tmp.path().join("memory/MEMORY.md");
    assert!(
        memory_md_path.exists(),
        "MEMORY.md should exist after dreaming"
    );
    let memory_content = std::fs::read_to_string(&memory_md_path).unwrap();
    assert!(
        memory_content.contains("- "),
        "MEMORY.md should contain at least one rule"
    );

    // Step 6: Verify dreaming status was updated.
    let statuses = storage.dreaming_statuses.lock().unwrap();
    assert!(
        statuses
            .iter()
            .any(|(sid, s)| sid == "sess-1" && *s == DreamingStatus::Completed),
        "dreaming status should be Completed"
    );
}

/// Edge case: empty transcript produces no events.
#[tokio::test]
async fn test_empty_transcript_no_events() {
    let (_tmp, miner, _dreaming) = setup(Box::new(MockMinerLlm::empty()));
    let storage = Arc::new(TestPersistence::default());

    // Checkpoint with no pending messages (empty transcript).
    let mut cp = SessionCheckpoint::new("sess-empty".into());
    cp.mined = false;
    cp.agent_id = Some("test-agent".into());
    cp.outbound_pending = Vec::new();
    storage.add_active(cp.clone());
    storage.add_archived(cp);

    let archived = storage
        .load_archived_checkpoint("sess-empty")
        .await
        .unwrap()
        .unwrap();
    let result = miner
        .mine_session_from_checkpoint("sess-empty", "", "test-agent", &archived, storage.as_ref())
        .await
        .unwrap();

    assert!(
        result.events.is_empty(),
        "empty transcript should produce no events"
    );
    assert!(result.entity_names.is_empty());
}

/// Edge case: LLM failure during mining degrades gracefully.
#[tokio::test]
async fn test_llm_failure_during_mining() {
    let (_tmp, miner, _dreaming) =
        setup_with_config(Box::new(MockMinerLlm::failing()), test_miner_config());
    let storage = Arc::new(TestPersistence::default());

    let transcript = vec![("user", "hello"), ("assistant", "hi")];
    let cp = make_archived_checkpoint("sess-fail", &transcript);
    storage.add_active(cp.clone());
    storage.add_archived(cp);

    let archived = storage
        .load_archived_checkpoint("sess-fail")
        .await
        .unwrap()
        .unwrap();
    let raw_transcript = format_transcript(&archived.outbound_pending);
    let result = miner
        .mine_session_from_checkpoint(
            "sess-fail",
            &raw_transcript,
            "test-agent",
            &archived,
            storage.as_ref(),
        )
        .await;

    assert!(result.is_err(), "LLM failure should propagate as error");
}

/// Edge case: duplicate events are deduplicated by SQLite UNIQUE constraints.
#[tokio::test]
async fn test_duplicate_events_deduplicated() {
    let llm = MockMinerLlm {
        events: vec![
            MiningEvent {
                title: "Test dedup event".into(),
                summary: "Same event testing".into(),
                body: "Same body".into(),
                category: MiningEventCategory::Decision,
                lesson: None,
            },
            MiningEvent {
                title: "Test dedup event".into(),
                summary: "Same event testing".into(),
                body: "Same body".into(),
                category: MiningEventCategory::Decision,
                lesson: None,
            },
        ],
        entities: vec![
            vec![MiningEntity {
                entity_type: "subject".into(),
                name: "same event testing".into(),
                description: "dedup".into(),
            }],
            vec![MiningEntity {
                entity_type: "subject".into(),
                name: "same event testing".into(),
                description: "dedup".into(),
            }],
        ],
        fail_extract: false,
        fail_assign: false,
    };

    let (tmp, miner, _dreaming) = setup_with_config(Box::new(llm), test_miner_config());
    let storage = Arc::new(TestPersistence::default());

    let transcript = vec![("user", "hello"), ("assistant", "hi")];
    let cp = make_archived_checkpoint("sess-dup", &transcript);
    storage.add_active(cp.clone());
    storage.add_archived(cp);

    let archived = storage
        .load_archived_checkpoint("sess-dup")
        .await
        .unwrap()
        .unwrap();
    let raw_transcript = format_transcript(&archived.outbound_pending);
    let result = miner
        .mine_session_from_checkpoint(
            "sess-dup",
            &raw_transcript,
            "test-agent",
            &archived,
            storage.as_ref(),
        )
        .await
        .unwrap();

    // Both events should be written (miner doesn't dedup at write time;
    // SQLite UNIQUE constraints handle entity dedup).
    assert_eq!(result.events.len(), 2);

    // Entity should be deduplicated (UNIQUE constraint on agent_id + type + normalized_name).
    let db_path = tmp.path().join("memory/memory.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let entity_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();
    assert_eq!(entity_count, 1, "entity should be deduplicated");
}

/// Edge case: already-mined session is skipped.
#[tokio::test]
async fn test_already_mined_session_skipped() {
    let (_tmp, miner, _dreaming) = setup(Box::new(MockMinerLlm::single_error_event()));
    let storage = Arc::new(TestPersistence::default());

    let transcript = vec![("user", "hello")];
    let mut cp = make_archived_checkpoint("sess-mined", &transcript);
    cp.mined = true;
    storage.add_active(cp.clone());
    storage.add_archived(cp);

    let archived = storage
        .load_archived_checkpoint("sess-mined")
        .await
        .unwrap()
        .unwrap();
    let raw_transcript = format_transcript(&archived.outbound_pending);
    let result = miner
        .mine_session_from_checkpoint(
            "sess-mined",
            &raw_transcript,
            "test-agent",
            &archived,
            storage.as_ref(),
        )
        .await
        .unwrap();

    assert!(
        result.events.is_empty(),
        "already-mined session should produce no events"
    );
}

/// Edge case: dreaming with no mined sessions is a no-op.
#[tokio::test]
async fn test_dreaming_no_mined_sessions_noop() {
    let (_tmp, _miner, dreaming) = setup(Box::new(MockMinerLlm::empty()));
    let storage = Arc::new(TestPersistence::default());

    // No sessions at all — dreaming should succeed without error.
    let result = dreaming.run_once(storage.as_ref()).await;
    assert!(result.is_ok(), "dreaming with no sessions should succeed");
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Format pending messages into raw transcript text.
fn format_transcript(messages: &[closeclaw_session::persistence::PendingMessage]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = if m.message_id.is_empty() {
                "unknown"
            } else {
                &m.message_id
            };
            format!("{role}: {}", m.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
