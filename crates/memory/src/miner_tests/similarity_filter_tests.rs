use crate::miner::{load_entity_type_thresholds, MemoryMiner, MinerConfig};
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

// ── Similarity threshold load tests ──────────────────────────────────

#[test]
fn test_load_entity_type_thresholds_all_types() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    let thresholds = load_entity_type_thresholds(&conn).unwrap();
    assert_eq!(thresholds.len(), 11);
    assert_eq!(thresholds["time"], 0.90);
    assert_eq!(thresholds["tags"], 0.70);
}

#[test]
fn test_load_entity_type_thresholds_excludes_inactive() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    conn.execute(
        "UPDATE entity_types SET is_active = 0 WHERE type = 'time'",
        [],
    )
    .unwrap();
    let thresholds = load_entity_type_thresholds(&conn).unwrap();
    assert!(!thresholds.contains_key("time"));
    assert!(thresholds.contains_key("tags"));
}

// ── Miner session similarity filter tests ────────────────────────────

/// High-threshold entity filtered when loosely related to event.
#[tokio::test]
async fn test_mine_session_filters_high_threshold_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-ht".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("January 2025", "time")]];
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
    let db_path = tmp.path().join("ht.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-ht", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        0,
        "high-threshold time entity should be filtered"
    );
}

/// Low-threshold entity retained when similar to event.
#[tokio::test]
async fn test_mine_session_keeps_low_threshold_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-lt".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Rust language", "tags")]];
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
    let db_path = tmp.path().join("lt.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-lt", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "low-threshold tags entity should be retained"
    );
}

/// Exact-match entity passes threshold boundary.
#[tokio::test]
async fn test_mine_session_boundary_at_threshold_keeps_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-bt".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Rust language basics", "subject")]];
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
    let db_path = tmp.path().join("bt.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-bt", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "exact-match entity should pass threshold"
    );
}

/// All entities filtered → empty entity_names.
#[tokio::test]
async fn test_mine_session_all_entities_filtered() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-all".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event("X", crate::miner::MiningEventCategory::Error)];
    let entities = vec![vec![
        make_entity("January 2025", "time"),
        make_entity("March 2026", "time"),
        make_entity("December 2024", "time"),
    ]];
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
    let db_path = tmp.path().join("all.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-all", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        0,
        "all loosely-related entities should be filtered"
    );
}
