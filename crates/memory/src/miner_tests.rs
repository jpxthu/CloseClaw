//! Unit tests for the memory miner.
//!
//! Covers transcript cleaning, Miner 1 extraction, Miner 2 entity
//! assignment, dedup logic, SQLite write operations, and edge cases.

use crate::miner::{
    load_entity_catalog, load_recent_events, normalize_entity_name, write_to_sqlite, MinerConfig,
    MiningEntity, MiningEvent, MiningEventCategory,
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
    let miner =
        crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md", "a1");

    let result = miner
        .mine_session("sess-1", "Owner: hi\nAgent: bye", &storage)
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
    let miner =
        crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md", "a1");

    let result = miner
        .mine_session("sess-1", "Owner: hi\nAgent: bye", &storage)
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
    let miner =
        crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md", "a1");

    let result = miner
        .mine_session("sess-1", "Owner: hi", &storage)
        .await
        .unwrap();
    assert!(result.events.is_empty());
}

#[tokio::test]
async fn test_mine_session_nonexistent_returns_error() {
    let storage = TestStorage::default();
    let config = MinerConfig::default();
    let llm = Box::new(MockMinerLlmCaller::default());
    let tmp = TempDir::new().unwrap();
    let miner =
        crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md", "a1");

    let result = miner
        .mine_session("does-not-exist", "Owner: hi\nAgent: bye", &storage)
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
    let miner = crate::miner::MemoryMiner::new(config, llm, &db_path, "memory.md", "a1");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", &storage)
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
        max_events_per_session: 5,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let miner =
        crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md", "a1");

    let result = miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", &storage)
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
    let lines: Vec<&str> = catalog.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("action"));
    assert!(lines[0].contains("Alpha"));
    assert!(lines[1].contains("subject"));
    assert!(lines[1].contains("Apple"));
    assert!(lines[2].contains("subject"));
    assert!(lines[2].contains("Zebra"));
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
    assert!(catalog_a1.contains("Entity A1"));
    assert!(!catalog_a1.contains("Entity A2"));

    let catalog_a2 = load_entity_catalog(&conn, "a2").unwrap();
    assert!(!catalog_a2.contains("Entity A1"));
    assert!(catalog_a2.contains("Entity A2"));
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
    let miner = crate::miner::MemoryMiner::new(config, llm, &db_path, "memory.md", "a1");

    miner
        .mine_session("sess-1", "Owner: hello\nAgent: response", &storage)
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
    assert!(config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}
