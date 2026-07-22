use crate::miner::{load_entity_catalog, load_recent_events, write_to_sqlite, MiningEventCategory};

use rusqlite::params;
use tempfile::TempDir;

use super::{make_entity, make_event};

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

    let event = crate::miner::MiningEvent {
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
    let action_pos = catalog.find("## action (动作):").unwrap();
    let subject_pos = catalog.find("## subject (主题):").unwrap();
    assert!(
        action_pos < subject_pos,
        "action should come before subject"
    );
    let apple_pos = catalog.find("- Apple: ap").unwrap();
    let zebra_pos = catalog.find("- Zebra: z").unwrap();
    assert!(apple_pos < zebra_pos, "Apple should come before Zebra");
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
    let result = load_recent_events(&conn, "other", "agent-1", 30).unwrap();
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
         category, lesson, source_session_id, agent_id, timestamp)
         VALUES ('Bug Fix', 'Fixed a bug', 'body',
         'error', 'lesson', 'other', 'agent-1', ?1)",
        params![ts],
    )
    .unwrap();

    let result = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
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
         category, lesson, source_session_id, agent_id, timestamp)
         VALUES ('old', 'old', 'body',
         'decision', NULL, 'other', 'agent-1', ?1)",
        params![old_ts],
    )
    .unwrap();
    let result = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
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
         category, lesson, source_session_id, agent_id, timestamp)
         VALUES ('Own Event', 'summary', 'body',
         'error', NULL, 'my-sess', 'agent-1', ?1)",
        params![ts],
    )
    .unwrap();

    let result = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
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
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'rust', 'rust', 'a language')",
        [],
    )
    .unwrap();
    let catalog = load_entity_catalog(&conn, "a1").unwrap();
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
    assert!(catalog.contains("- rust: a language"));
}

/// Inactive types (is_active = 0) should not appear in the catalog.
#[test]
fn test_load_entity_catalog_excludes_inactive_types() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    conn.execute("UPDATE entity_types SET is_active = 0 WHERE id = 11", [])
        .unwrap();
    let catalog = load_entity_catalog(&conn, "a1").unwrap();
    assert!(
        !catalog.contains("## tags "),
        "inactive type 'tags' should not appear in catalog",
    );
    assert!(catalog.contains("## subject "));
    assert!(catalog.contains("## action "));
}
