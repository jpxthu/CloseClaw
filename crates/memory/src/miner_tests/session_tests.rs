use crate::miner::{MemoryMiner, MinerConfig};
use crate::miner_llm::MockMinerLlmCaller;
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::TranscriptCleanRules;
use closeclaw_session::persistence::SessionCheckpoint;

use tempfile::TempDir;

use super::{make_entity, make_event};

/// Lenient rules: 1 turn, 1 owner message, md format.
fn lenient_rules() -> TranscriptCleanRules {
    TranscriptCleanRules {
        min_turns: Some(1),
        min_owner_msgs: Some(1),
        format: Some("md".to_string()),
    }
}

// ── Mine session tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_mine_session_skips_when_disabled() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let config = MinerConfig {
        enabled: false,
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller::default());
    let tmp = TempDir::new().unwrap();
    let miner = MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hi\nAgent: bye", "a1", &storage)
        .await
        .unwrap();
    assert!(result.events.is_empty());
}

#[tokio::test]
async fn test_mine_session_skips_already_mined() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = true;
    storage.add_checkpoint(cp);

    let config = MinerConfig::default();
    let llm = Box::new(MockMinerLlmCaller {
        events_response: vec![make_event(
            "should not appear",
            crate::miner::MiningEventCategory::Error,
        )],
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner = MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hi\nAgent: bye", "a1", &storage)
        .await
        .unwrap();
    assert!(result.events.is_empty());
}

#[tokio::test]
async fn test_mine_session_empty_transcript() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let config = MinerConfig::default();
    let llm = Box::new(MockMinerLlmCaller {
        events_response: vec![make_event(
            "should not appear",
            crate::miner::MiningEventCategory::Error,
        )],
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner = MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hi", "a1", &storage)
        .await
        .unwrap();
    assert!(result.events.is_empty());
}

#[tokio::test]
async fn test_mine_session_nonexistent_returns_error() {
    let storage = TestStorage::default();
    let config = MinerConfig {
        enabled: true,
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller::default());
    let tmp = TempDir::new().unwrap();
    let miner = MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("does-not-exist", "Owner: hi\nAgent: bye", "a1", &storage)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_mine_session_happy_path() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let events = vec![make_event(
        "Test Entity",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Test Entity", "subject")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("mining.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].title, "Test Entity");
    assert_eq!(result.entity_names[0], vec!["Test Entity".to_string()]);

    let mined = storage.mined_ids();
    assert!(mined.contains(&"sess-1".to_string()));
}

#[tokio::test]
async fn test_mine_session_respects_max_events_limit() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let events: Vec<_> = (0..20)
        .map(|i| {
            make_event(
                &format!("event {i}"),
                crate::miner::MiningEventCategory::Decision,
            )
        })
        .collect();
    let config = MinerConfig {
        enabled: true,
        max_events_per_session: 5,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner = MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 5, "should truncate to max_events");
}

// ── Integration: mine_session writes to SQLite ────────────────────────

#[tokio::test]
async fn test_mine_session_persists_to_sqlite() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let events = vec![make_event(
        "persisted event",
        crate::miner::MiningEventCategory::Decision,
    )];
    let entities = vec![vec![make_entity("Persisted Entity", "subject")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("persist.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");

    miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let title: String = conn
        .query_row("SELECT title FROM events LIMIT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(title, "persisted event");
}
