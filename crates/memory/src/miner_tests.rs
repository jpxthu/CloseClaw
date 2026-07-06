//! Unit tests for the memory miner.
//!
//! Covers transcript cleaning, Miner 1 extraction, Miner 2 entity
//! assignment, dedup logic, SQLite write operations, and edge cases.

use crate::miner::{
    load_entity_catalog, load_recent_events, normalize_entity_name, write_to_sqlite, MemoryMiner,
    MinerConfig, MiningEntity, MiningEvent, MiningEventCategory,
};
use crate::miner_llm::MockMinerLlmCaller;
use crate::miner_transcript::clean_transcript;
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::{MiningConfig, TranscriptCleanRules};
use closeclaw_session::persistence::SessionCheckpoint;

use rusqlite::params;
use tempfile::TempDir;

// ── Helpers ────────────────────────────────────────────────────────────
fn make_event(title: &str, category: MiningEventCategory) -> MiningEvent {
    let has_lesson = category != MiningEventCategory::Decision;
    MiningEvent {
        title: title.to_string(),
        summary: format!("Summary of {title}"),
        body: format!("Body of {title}"),
        category,
        lesson: if has_lesson {
            Some(format!("Lesson from {title}"))
        } else {
            None
        },
    }
}

fn make_entity(name: &str, typ: &str) -> MiningEntity {
    MiningEntity {
        entity_type: typ.to_string(),
        name: name.to_string(),
        description: format!("Desc of {name}"),
    }
}

/// Lenient rules: 1 turn, 1 owner message, md format.
fn lenient_rules() -> TranscriptCleanRules {
    TranscriptCleanRules {
        min_turns: Some(1),
        min_owner_msgs: Some(1),
        format: Some("md".to_string()),
    }
}

fn make_transcript(n_owner: usize, n_agent: usize) -> String {
    let mut lines = Vec::new();
    for i in 0..n_owner {
        lines.push(format!("Owner: owner message {i}"));
        if i < n_agent {
            lines.push(format!("Agent: agent response {i}"));
        }
    }
    lines.join("\n")
}
// ── Transcript cleaning tests ─────────────────────────────────────────

#[test]
fn test_transcript_clean_removes_thinking_blocks() {
    let raw = "Owner: hello\n<thinking>\nSome thought\n</thinking>\nAgent: hi";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("thinking"));
    assert!(!cleaned.contains("Some thought"));
    assert!(cleaned.contains("hello"));
    assert!(cleaned.contains("hi"));
}

#[test]
fn test_transcript_clean_removes_tool_call_xml() {
    let raw = "Owner: go\n<tool_call>{\"name\":\"exec\"}</tool_call>\nAgent: done";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("tool_call"));
    assert!(cleaned.contains("done"));
}

#[test]
fn test_transcript_clean_removes_context_markers() {
    let raw = "Owner: test\n[context: system prompt]\nAgent: ok";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("[context:"));
}

#[test]
fn test_transcript_clean_removes_media_no_reply() {
    let raw = "Owner: msg\nMEDIA\nNO_REPLY\nAgent: response";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("MEDIA"));
    assert!(!cleaned.contains("NO_REPLY"));
}

#[test]
fn test_transcript_clean_collapses_blank_lines() {
    let raw = "Owner: a\n\n\n\nAgent: b";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("\n\n\n"));
}

#[test]
fn test_transcript_clean_skips_short_transcripts() {
    let raw = "Owner: hi";
    let rules = TranscriptCleanRules {
        min_turns: Some(5),
        min_owner_msgs: Some(5),
        format: Some("md".to_string()),
    };
    let cleaned = clean_transcript(raw, &rules);
    assert!(cleaned.is_empty());
}

#[test]
fn test_transcript_clean_skips_few_owner_messages() {
    let raw = make_transcript(2, 5);
    let rules = TranscriptCleanRules {
        min_owner_msgs: Some(10),
        ..Default::default()
    };
    let cleaned = clean_transcript(&raw, &rules);
    assert!(cleaned.is_empty());
}

#[test]
fn test_transcript_clean_normalizes_owner_prefixes() {
    let raw = "Owner: a\nuser: b\nowner: c";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(cleaned.contains("a"));
    assert!(cleaned.contains("b"));
    assert!(cleaned.contains("c"));
}
// ── MinerConfig tests ─────────────────────────────────────────────────

#[test]
fn test_miner_config_from_mining_config() {
    let mc = MiningConfig {
        enabled: Some(true),
        max_events_per_session: Some(15),
        dedup_window_days: Some(60),
        transcript_clean_rules: TranscriptCleanRules {
            min_turns: Some(3),
            min_owner_msgs: Some(4),
            format: Some("plain".to_string()),
        },
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert!(config.enabled);
    assert_eq!(config.max_events_per_session, 15);
    assert_eq!(config.dedup_window_days, 60);
    assert_eq!(config.clean_rules.min_turns, Some(3));
}

#[test]
fn test_miner_config_defaults() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
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
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

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
        events_response: vec![make_event("should not appear", MiningEventCategory::Error)],
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

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
        events_response: vec![make_event("should not appear", MiningEventCategory::Error)],
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

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
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

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

    let events = vec![make_event("error event", MiningEventCategory::Error)];
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
    let miner = crate::miner::MemoryMiner::new(config, llm, &db_path, "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].title, "error event");
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

    let events: Vec<MiningEvent> = (0..20)
        .map(|i| make_event(&format!("event {i}"), MiningEventCategory::Decision))
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
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 5, "should truncate to max_events");
}
// ── SQLite write tests ────────────────────────────────────────────────

#[test]
fn test_write_to_sqlite_creates_events() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let events = vec![make_event("test event", MiningEventCategory::Error)];
    let entities = vec![vec![make_entity("My Entity", "subject")]];

    write_to_sqlite(&conn, "sess-1", "a1", &events, &entities).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entity_count, 1);

    let link_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(link_count, 1);
}

#[test]
fn test_write_to_sqlite_deduplicates_entities() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let events = vec![
        make_event("event 1", MiningEventCategory::Error),
        make_event("event 2", MiningEventCategory::Anger),
    ];
    let entities = vec![
        vec![make_entity("Same Entity", "subject")],
        vec![make_entity("Same Entity", "subject")],
    ];

    write_to_sqlite(&conn, "sess-1", "a1", &events, &entities).unwrap();

    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entity_count, 1, "same entity should not be duplicated");

    let link_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(link_count, 2, "each event should link to the entity");
}

#[test]
fn test_write_to_sqlite_stores_event_fields() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let event = MiningEvent {
        title: "My Title".to_string(),
        summary: "My Summary".to_string(),
        body: "My Body".to_string(),
        category: MiningEventCategory::Anger,
        lesson: Some("My Lesson".to_string()),
    };
    write_to_sqlite(&conn, "sess-1", "a1", &[event], &[vec![]]).unwrap();

    let title: String = conn
        .query_row("SELECT title FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let category: String = conn
        .query_row("SELECT category FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let lesson: Option<String> = conn
        .query_row("SELECT lesson FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();

    assert_eq!(title, "My Title");
    assert_eq!(category, "anger");
    assert_eq!(lesson.as_deref(), Some("My Lesson"));
}
// ── Entity catalog tests ──────────────────────────────────────────────

#[test]
fn test_load_entity_catalog_sorts_by_type_then_name() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'Zebra', 'zebra', 'z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'action', 'Alpha', 'alpha', 'a')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'Apple', 'apple', 'ap')",
        [],
    )
    .unwrap();

    let catalog = load_entity_catalog(&conn, "a1").unwrap();
    // Types are sorted alphabetically: action before subject.
    let action_pos = catalog.find("## action (动作):").unwrap();
    let subject_pos = catalog.find("## subject (主题):").unwrap();
    assert!(
        action_pos < subject_pos,
        "action should come before subject"
    );
    // Entities within subject type are sorted by normalized_name.
    let apple_pos = catalog.find("- Apple: ap").unwrap();
    let zebra_pos = catalog.find("- Zebra: z").unwrap();
    assert!(apple_pos < zebra_pos, "Apple should come before Zebra");
    // Action entity appears under action header.
    assert!(catalog.contains("- Alpha: a"));
}

#[test]
fn test_load_entity_catalog_scoped_by_agent() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'Entity A1', 'entity_a1', '')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a2', 'subject', 'Entity A2', 'entity_a2', '')",
        [],
    )
    .unwrap();

    let catalog_a1 = load_entity_catalog(&conn, "a1").unwrap();
    assert!(catalog_a1.contains("- Entity A1:"));
    assert!(!catalog_a1.contains("Entity A2"));

    let catalog_a2 = load_entity_catalog(&conn, "a2").unwrap();
    assert!(!catalog_a2.contains("Entity A1"));
    assert!(catalog_a2.contains("- Entity A2:"));
}
// ── Recent events load tests ──────────────────────────────────────────

#[test]
fn test_load_recent_events_empty_db() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    let result = load_recent_events(&conn, "other", 30).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_load_recent_events_with_data() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let ts = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events (title, summary, content,
         category, lesson, source_session_id, timestamp)
         VALUES ('Bug Fix', 'Fixed a bug', 'body',
         'error', 'lesson', 'other', ?1)",
        params![ts],
    )
    .unwrap();

    let result = load_recent_events(&conn, "my-sess", 30).unwrap();
    assert!(result.contains("[error]"));
    assert!(result.contains("Bug Fix"));
    assert!(result.contains("Fixed a bug"));
}

#[test]
fn test_load_recent_events_excludes_old() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let old_ts = chrono::Utc::now().timestamp() - (60 * 86400);
    conn.execute(
        "INSERT INTO events (title, summary, content,
         category, lesson, source_session_id, timestamp)
         VALUES ('old', 'old', 'body',
         'decision', NULL, 'other', ?1)",
        params![old_ts],
    )
    .unwrap();
    let result = load_recent_events(&conn, "my-sess", 30).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_load_recent_events_excludes_current_session() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let ts = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events (title, summary, content,
         category, lesson, source_session_id, timestamp)
         VALUES ('Own Event', 'summary', 'body',
         'error', NULL, 'my-sess', ?1)",
        params![ts],
    )
    .unwrap();

    let result = load_recent_events(&conn, "my-sess", 30).unwrap();
    assert!(result.is_empty());
}
// ── entity_types table tests ──────────────────────────────────────────

/// init_schema creates entity_types table with 11 seed rows.
#[test]
fn test_init_schema_creates_entity_types() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entity_types", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 11, "entity_types should have 11 seed rows");
}

/// catalog includes type definitions for all 11 types.
#[test]
fn test_load_entity_catalog_includes_type_definitions() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    // Insert an entity to verify the catalog merges both tables.
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'rust', 'rust', 'a language')",
        [],
    )
    .unwrap();
    let catalog = load_entity_catalog(&conn, "a1").unwrap();
    // All 11 type headers must be present.
    let expected_types = [
        "action",
        "group",
        "location",
        "metric",
        "organization",
        "person",
        "product",
        "subject",
        "tags",
        "time",
        "work",
    ];
    for t in expected_types {
        assert!(
            catalog.contains(&format!("## {t} ")),
            "catalog should contain type header for {t}",
        );
    }
    // Entity should also appear.
    assert!(catalog.contains("- rust: a language"));
}

/// Inactive types (is_active = 0) should not appear in the catalog.
#[test]
fn test_load_entity_catalog_excludes_inactive_types() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    // Deactivate the 'tags' type (id=11 in seed data).
    conn.execute("UPDATE entity_types SET is_active = 0 WHERE id = 11", [])
        .unwrap();
    let catalog = load_entity_catalog(&conn, "a1").unwrap();
    assert!(
        !catalog.contains("## tags "),
        "inactive type 'tags' should not appear in catalog",
    );
    // Active types should still be present.
    assert!(catalog.contains("## subject "));
    assert!(catalog.contains("## action "));
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
// ── Per-agent entity isolation tests ─────────────────────────────────

/// Mining with different agent_id values produces isolated entity catalogs.
#[tokio::test]
async fn test_per_agent_isolation_different_agent_ids() {
    let storage = TestStorage::default();
    let mut cp_a = SessionCheckpoint::new("sess-a".into());
    cp_a.mined = false;
    storage.add_checkpoint(cp_a);
    let mut cp_b = SessionCheckpoint::new("sess-b".into());
    cp_b.mined = false;
    storage.add_checkpoint(cp_b);

    let events_a = vec![make_event("agent-A event", MiningEventCategory::Error)];
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

    // Mine a different session with agent-B producing a different entity.
    let events_b = vec![make_event("agent-B event", MiningEventCategory::Error)];
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

    // Verify isolation: each agent's catalog contains only its own entity.
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

    let events = vec![make_event(
        "empty agent event",
        MiningEventCategory::Decision,
    )];
    let entities = vec![vec![make_entity("Empty Agent Entity", "action")]];
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
        catalog.contains("- Empty Agent Entity:"),
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
    // Both events assign the same entity with same agent_id.
    let entities = vec![
        vec![make_entity("Shared Entity", "subject")],
        vec![make_entity("Shared Entity", "subject")],
    ];

    write_to_sqlite(&conn, "sess-1", "agent-A", &events, &entities).unwrap();
    // Only one entity row should exist (UNIQUE constraint).
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        entity_count, 1,
        "duplicate entity for same agent_id should be deduped"
    );
    // Both events should link to that single entity.
    let link_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(link_count, 2);
}
// ── MiningEventCategory Display ───────────────────────────────────────

#[test]
fn test_mining_event_category_display() {
    assert_eq!(MiningEventCategory::Error.to_string(), "error");
    assert_eq!(MiningEventCategory::Anger.to_string(), "anger");
    assert_eq!(MiningEventCategory::Decision.to_string(), "decision");
}
// ── Integration: mine_session writes to SQLite ────────────────────────

#[tokio::test]
async fn test_mine_session_persists_to_sqlite() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let events = vec![make_event("persisted event", MiningEventCategory::Decision)];
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
    let miner = crate::miner::MemoryMiner::new(config, llm, &db_path, "memory.md");

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
// ── Config hot-reload tests ────────────────────────────────────────

/// update_config reflects new enabled value in is_enabled().
#[test]
fn test_update_config_reflects_new_enabled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = MinerConfig {
        enabled: false,
        ..Default::default()
    };
    let llm = Box::new(crate::miner_llm::MockMinerLlmCaller::default());
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");
    assert!(!miner.is_enabled(), "should start disabled");

    // Hot-reload: enable mining.
    let new_config = MinerConfig {
        enabled: true,
        ..Default::default()
    };
    miner.update_config(new_config);
    assert!(miner.is_enabled(), "should be enabled after update_config");
}
// ── MinerConfig model propagation tests ───────────────────────────────

/// from_mining_config() copies model from MiningConfig.
#[test]
fn test_miner_config_from_mining_config_copies_model() {
    let mc = MiningConfig {
        model: Some("gpt-4o-mini".to_string()),
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model.as_deref(), Some("gpt-4o-mini"));
}

/// from_mining_config() propagates None model as None.
#[test]
fn test_miner_config_from_mining_config_none_model() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model, None);
}

/// from_mining_config() preserves empty string model.
#[test]
fn test_miner_config_from_mining_config_empty_string_model() {
    let mc = MiningConfig {
        model: Some(String::new()),
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model.as_deref(), Some(""));
}

/// Default MinerConfig has model as None.
#[test]
fn test_miner_config_default_model_is_none() {
    let config = MinerConfig::default();
    assert_eq!(config.model, None);
}
// ── MemoryMiner model getter tests ────────────────────────────────────

fn make_miner(config: MinerConfig) -> MemoryMiner {
    let tmp = tempfile::TempDir::new().unwrap();
    let llm = Box::new(crate::miner_llm::MockMinerLlmCaller::default());
    crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md")
}

/// model() returns None when no model is configured.
#[test]
fn test_model_returns_none_when_unconfigured() {
    let miner = make_miner(MinerConfig::default());
    assert_eq!(miner.model(), None, "model should be None by default");
}

/// model() returns the configured model name.
#[test]
fn test_model_returns_configured_value() {
    let config = MinerConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some("gpt-4o"));
}

/// model() returns empty string when configured as empty.
#[test]
fn test_model_returns_empty_string() {
    let config = MinerConfig {
        model: Some(String::new()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some(""));
}

/// update_config propagates new model to getter.
#[test]
fn test_update_config_propagates_model() {
    let miner = make_miner(MinerConfig::default());
    assert_eq!(miner.model(), None);

    let new_config = MinerConfig {
        model: Some("claude-3.5-sonnet".to_string()),
        ..Default::default()
    };
    miner.update_config(new_config);
    assert_eq!(miner.model().as_deref(), Some("claude-3.5-sonnet"));
}

/// update_config can set model from Some to None.
#[test]
fn test_update_config_clears_model() {
    let config = MinerConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some("gpt-4o"));

    let new_config = MinerConfig {
        model: None,
        ..Default::default()
    };
    miner.update_config(new_config);
    assert_eq!(miner.model(), None);
}

/// Per-agent override: different miners can have different models.
#[test]
fn test_per_agent_override_model() {
    let miner_a = make_miner(MinerConfig {
        model: Some("model-a".to_string()),
        ..Default::default()
    });
    let miner_b = make_miner(MinerConfig {
        model: Some("model-b".to_string()),
        ..Default::default()
    });
    assert_eq!(miner_a.model().as_deref(), Some("model-a"));
    assert_eq!(miner_b.model().as_deref(), Some("model-b"));
}
// ── MinerConfig from_mining_config edge cases ─────────────────────────

#[test]
fn test_miner_config_from_mining_config_none_values() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}

#[test]
fn test_miner_config_default_values() {
    let config = MinerConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}
