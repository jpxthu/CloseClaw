use crate::miner::{
    load_entity_catalog, normalize_entity_name, write_to_sqlite, MemoryMiner, MinerConfig,
    MiningEventCategory,
};
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

// ── normalize_entity_name tests ───────────────────────────────────────

#[test]
fn test_normalize_entity_name_various() {
    assert_eq!(normalize_entity_name("Hello World"), "hello_world");
    assert_eq!(normalize_entity_name("ALL CAPS"), "all_caps");
    assert_eq!(normalize_entity_name("already_lower"), "already_lower");
    assert_eq!(
        normalize_entity_name("Multiple   Spaces"),
        "multiple___spaces"
    );
}

// ── MiningEventCategory Display ───────────────────────────────────────

#[test]
fn test_mining_event_category_display() {
    assert_eq!(MiningEventCategory::Error.to_string(), "error");
    assert_eq!(MiningEventCategory::Anger.to_string(), "anger");
    assert_eq!(MiningEventCategory::Decision.to_string(), "decision");
}

// ── Per-agent entity isolation tests ─────────────────────────────────

#[tokio::test]
async fn test_per_agent_isolation_different_agent_ids() {
    let storage = TestStorage::default();
    let mut cp_a = SessionCheckpoint::new("sess-a".into());
    cp_a.mined = false;
    storage.add_checkpoint(cp_a);
    let mut cp_b = SessionCheckpoint::new("sess-b".into());
    cp_b.mined = false;
    storage.add_checkpoint(cp_b);

    let events_a = vec![make_event("Entity From A", MiningEventCategory::Error)];
    let entities_a = vec![vec![make_entity("Entity From A", "subject")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm_a = Box::new(MockMinerLlmCaller {
        events_response: events_a,
        entities_response: entities_a,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("isolation.db");
    let miner_a = MemoryMiner::new(config.clone(), llm_a, &db_path, "memory.md");
    miner_a
        .mine_session(
            "sess-a",
            "Owner: hello\nAgent: response",
            "agent-A",
            &storage,
        )
        .await
        .unwrap();

    let events_b = vec![make_event("Entity From B", MiningEventCategory::Error)];
    let entities_b = vec![vec![make_entity("Entity From B", "subject")]];
    let llm_b = Box::new(MockMinerLlmCaller {
        events_response: events_b,
        entities_response: entities_b,
        ..Default::default()
    });
    let miner_b = MemoryMiner::new(config, llm_b, &db_path, "memory.md");
    miner_b
        .mine_session(
            "sess-b",
            "Owner: hello\nAgent: response",
            "agent-B",
            &storage,
        )
        .await
        .unwrap();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let catalog_a = load_entity_catalog(&conn, "agent-A").unwrap();
    assert!(
        catalog_a.contains("- Entity From A:"),
        "agent-A should have its own entity"
    );
    assert!(
        !catalog_a.contains("Entity From B"),
        "agent-A must not see agent-B's entity"
    );

    let catalog_b = load_entity_catalog(&conn, "agent-B").unwrap();
    assert!(
        catalog_b.contains("- Entity From B:"),
        "agent-B should have its own entity"
    );
    assert!(
        !catalog_b.contains("Entity From A"),
        "agent-B must not see agent-A's entity"
    );
}

/// Empty agent_id should not panic.
#[tokio::test]
async fn test_per_agent_empty_agent_id_no_panic() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-empty".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let events = vec![make_event("Empty Agent", MiningEventCategory::Decision)];
    let entities = vec![vec![make_entity("Empty Agent", "action")]];
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
    let db_path = tmp.path().join("empty_agent.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");

    let result = miner
        .mine_session("sess-empty", "Owner: hi\nAgent: bye", "", &storage)
        .await;
    assert!(
        result.is_ok(),
        "mining with empty agent_id should not panic"
    );
    assert_eq!(result.unwrap().events.len(), 1);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let catalog = load_entity_catalog(&conn, "").unwrap();
    assert!(
        catalog.contains("- Empty Agent:"),
        "empty agent should have entity"
    );
}

/// Same agent_id + type + normalized_name must dedup via UNIQUE constraint.
#[test]
fn test_per_agent_dedup_same_agent_type_name() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("dedup.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let events = vec![
        make_event("event 1", MiningEventCategory::Error),
        make_event("event 2", MiningEventCategory::Error),
    ];
    let entities = vec![
        vec![make_entity("Shared Entity", "subject")],
        vec![make_entity("Shared Entity", "subject")],
    ];

    write_to_sqlite(&conn, "sess-1", "agent-A", &events, &entities).unwrap();
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        entity_count, 1,
        "duplicate entity for same agent_id should be deduped"
    );
    let link_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(link_count, 2);
}
