use crate::miner::{load_entity_catalog, load_recent_events, write_to_sqlite, MiningEventCategory};
use closeclaw_config::agents::default_forgetting_initial_ttl_days;

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

    write_to_sqlite(
        &conn,
        "sess-1",
        "a1",
        &events,
        &entities,
        default_forgetting_initial_ttl_days(),
        default_forgetting_initial_ttl_days(),
    )
    .unwrap();

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

    write_to_sqlite(
        &conn,
        "sess-1",
        "a1",
        &events,
        &entities,
        default_forgetting_initial_ttl_days(),
        default_forgetting_initial_ttl_days(),
    )
    .unwrap();

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
        reidentified_event_id: None,
    };
    write_to_sqlite(
        &conn,
        "sess-1",
        "a1",
        &[event],
        &[vec![]],
        default_forgetting_initial_ttl_days(),
        default_forgetting_initial_ttl_days(),
    )
    .unwrap();

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
    let (result, ids) = load_recent_events(&conn, "other", "agent-1", 30).unwrap();
    assert!(result.is_empty());
    assert!(ids.is_empty());
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

    let (result, ids) = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
    assert!(result.contains("[error]"));
    assert!(result.contains("Bug Fix"));
    assert!(result.contains("Fixed a bug"));
    assert_eq!(ids.len(), 1);
    assert!(ids[0] > 0);
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
    let (result, ids) = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
    assert!(result.is_empty());
    assert!(ids.is_empty());
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

    let (result, ids) = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
    assert!(result.is_empty());
    assert!(ids.is_empty());
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

// ── Insight category tests ─────────────────────────────────────────

/// MiningEventCategory::Insight Display should output "insight".
#[test]
fn test_insight_category_display() {
    let cat = MiningEventCategory::Insight;
    assert_eq!(cat.to_string(), "insight");
}

/// write_to_sqlite with an Insight event should persist category = "insight"
/// and lesson = NULL.
#[test]
fn test_write_to_sqlite_insight_category_and_lesson() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let event = crate::miner::MiningEvent {
        title: "insight title".to_string(),
        summary: "insight summary".to_string(),
        body: "insight body".to_string(),
        category: MiningEventCategory::Insight,
        lesson: None,
        reidentified_event_id: None,
    };
    write_to_sqlite(
        &conn,
        "sess-insight",
        "a1",
        &[event],
        &[vec![]],
        default_forgetting_initial_ttl_days(),
        default_forgetting_initial_ttl_days(),
    )
    .unwrap();

    let category: String = conn
        .query_row("SELECT category FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(category, "insight");

    let lesson: Option<String> = conn
        .query_row("SELECT lesson FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(lesson.is_none(), "Insight event lesson should be NULL");
}

/// write_entries_to_db with Insight event round-trips correctly.
#[test]
fn test_write_entries_to_db_insight_round_trip() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let event = crate::miner::MiningEvent {
        title: "discovered pattern".to_string(),
        summary: "retry loop yields rule".to_string(),
        body: "body".to_string(),
        category: MiningEventCategory::Insight,
        lesson: None,
        reidentified_event_id: None,
    };
    let entities = vec![vec![make_entity("retry_pattern", "subject")]];

    super::super::miner::write_entries_to_db(
        &conn,
        "sess-insight-round",
        "a1",
        &[event],
        &entities,
    )
    .unwrap();

    let category: String = conn
        .query_row("SELECT category FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(category, "insight");

    let lesson: Option<String> = conn
        .query_row("SELECT lesson FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(lesson.is_none());

    // Entity should exist
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(entity_count, 1);
}

// ── expires_at tests ────────────────────────────────────────────────

/// New event written via write_to_sqlite should have expires_at ≈ now + 90 * 86400.
#[test]
fn test_write_to_sqlite_sets_expires_at() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let events = vec![make_event("expiring event", MiningEventCategory::Error)];
    let entities = vec![vec![]];
    let ttl_days = default_forgetting_initial_ttl_days();
    write_to_sqlite(
        &conn, "sess-1", "a1", &events, &entities, ttl_days, ttl_days,
    )
    .unwrap();

    let now = chrono::Utc::now().timestamp();
    let expires_at: i64 = conn
        .query_row("SELECT expires_at FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();

    let expected = now + ttl_days * 86400;
    assert!(
        (expires_at - expected).abs() < 120,
        "expires_at {expires_at} should be ≈ {expected} (±120s)"
    );
}

/// Custom initial_ttl_days should be reflected in expires_at.
#[test]
fn test_write_to_sqlite_custom_ttl_days() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let events = vec![make_event("short ttl", MiningEventCategory::Error)];
    let entities = vec![vec![]];
    let custom_ttl: i64 = 30;
    write_to_sqlite(
        &conn, "sess-1", "a1", &events, &entities, custom_ttl, custom_ttl,
    )
    .unwrap();

    let now = chrono::Utc::now().timestamp();
    let expires_at: i64 = conn
        .query_row("SELECT expires_at FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();

    let expected = now + custom_ttl * 86400;
    assert!(
        (expires_at - expected).abs() < 120,
        "expires_at {expires_at} should be ≈ {expected} (±120s)"
    );
}

/// Migration: existing DB without expires_at column → column exists with default 0.
#[test]
fn test_init_schema_migration_adds_expires_at_to_existing_db() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();

    // Create an old-style events table without expires_at.
    conn.execute_batch(
        "CREATE TABLE events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            content TEXT NOT NULL,
            category TEXT NOT NULL,
            lesson TEXT,
            source_session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL DEFAULT '',
            timestamp INTEGER NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO events (title, summary, content, category, source_session_id, agent_id, timestamp)
        VALUES ('old', 'old', 'body', 'error', 'sess', 'agent-1', 1000000);",
    )
    .unwrap();

    // init_schema should add expires_at via ALTER TABLE.
    crate::miner::init_schema(&conn).unwrap();

    // Column should exist and existing row should have default value 0.
    let expires_at: i64 = conn
        .query_row("SELECT expires_at FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        expires_at, 0,
        "existing row should have expires_at = 0 (default)"
    );
}

/// Second call to init_schema does not error (idempotent).
#[test]
fn test_init_schema_idempotent_with_migration() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    // Second call should succeed without error.
    crate::miner::init_schema(&conn).unwrap();
}

/// events table DDL includes expires_at column.
#[test]
fn test_init_schema_events_has_expires_at_column() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let column_exists: bool = conn
        .prepare("SELECT expires_at FROM events WHERE 0")
        .is_ok();
    assert!(column_exists, "events table should have expires_at column");
}

// ── Re-identify extends expires_at tests ──────────────────────────────

/// write_to_sqlite with reidentified_event_id = Some(id) extends the
/// existing event's expires_at instead of inserting a new row.
#[test]
fn test_reidentify_extends_expires_at() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    // Insert an original event with a known expires_at.
    let original_ts = chrono::Utc::now().timestamp();
    let original_expires = original_ts + 30 * 86400; // 30 days from now
    let original_id: i64 = conn
        .query_row(
            "INSERT INTO events (title, summary, content, category, lesson,
             source_session_id, agent_id, timestamp, updated_at, expires_at)
             VALUES ('orig', 'orig', 'body', 'error', NULL, 'sess-1', 'a1',
             ?1, ?1, ?2)
             RETURNING id",
            params![original_ts, original_expires],
            |row| row.get(0),
        )
        .unwrap();

    // Re-identify: should UPDATE expires_at, not INSERT.
    let extension_days: i64 = 60;
    let reidentify_event = crate::miner::MiningEvent {
        title: "reoccurrence".to_string(),
        summary: "same event again".to_string(),
        body: "body".to_string(),
        category: MiningEventCategory::Error,
        lesson: None,
        reidentified_event_id: Some(original_id),
    };
    write_to_sqlite(
        &conn,
        "sess-2",
        "a1",
        &[reidentify_event],
        &[vec![]],
        90, // initial_ttl_days (not used for re-identify)
        extension_days,
    )
    .unwrap();

    // Should still be exactly 1 event (no new row inserted).
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "re-identify should not insert a new row");

    // expires_at should be extended: MAX(original_expires, now) + extension.
    let now = chrono::Utc::now().timestamp();
    let new_expires: i64 = conn
        .query_row(
            "SELECT expires_at FROM events WHERE id = ?1",
            params![original_id],
            |row| row.get(0),
        )
        .unwrap();
    let expected_base = now.max(original_expires);
    let expected = expected_base + extension_days * 86400;
    assert!(
        (new_expires - expected).abs() < 120,
        "expires_at should be MAX(now, original) + extension, got {new_expires}, expected ~{expected}"
    );
    assert!(
        new_expires > original_expires,
        "new expires_at {new_expires} should be > original {original_expires}"
    );
}

/// write_to_sqlite with reidentified_event_id = None still INSERTs normally.
#[test]
fn test_normal_insert_unchanged() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let event = crate::miner::MiningEvent {
        title: "normal".to_string(),
        summary: "summary".to_string(),
        body: "body".to_string(),
        category: MiningEventCategory::Decision,
        lesson: None,
        reidentified_event_id: None,
    };
    write_to_sqlite(&conn, "sess-normal", "a1", &[event], &[vec![]], 90, 90).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "None reidentified_event_id should INSERT");

    let title: String = conn
        .query_row("SELECT title FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(title, "normal");
}

/// Re-identifying M events out of N leaves total count at N.
#[test]
fn test_reidentify_event_count_unchanged() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let n: usize = 5;
    let m: usize = 3;

    // Insert N original events.
    let mut original_ids = Vec::new();
    let base_ts = chrono::Utc::now().timestamp();
    for i in 0..n {
        let id: i64 = conn
            .query_row(
                "INSERT INTO events (title, summary, content, category, lesson,
                 source_session_id, agent_id, timestamp, updated_at, expires_at)
                 VALUES (?1, ?1, 'body', 'error', NULL, 'sess-1', 'a1',
                 ?2, ?2, ?3)
                 RETURNING id",
                params![format!("event {}", i), base_ts, base_ts + 30 * 86400],
                |row| row.get(0),
            )
            .unwrap();
        original_ids.push(id);
    }

    // Re-identify the first M events.
    let reidentify_events: Vec<crate::miner::MiningEvent> = original_ids[..m]
        .iter()
        .enumerate()
        .map(|(i, &id)| crate::miner::MiningEvent {
            title: format!("reocc {}", i),
            summary: format!("reocc summary {}", i),
            body: "body".to_string(),
            category: MiningEventCategory::Error,
            lesson: None,
            reidentified_event_id: Some(id),
        })
        .collect();
    let reidentify_entities: Vec<Vec<crate::miner::MiningEntity>> =
        reidentify_events.iter().map(|_| vec![]).collect();

    write_to_sqlite(
        &conn,
        "sess-2",
        "a1",
        &reidentify_events,
        &reidentify_entities,
        90,
        60,
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, n as i64, "total event count should remain {n}");

    // Verify expires_at was extended on all M re-identified events.
    for &id in &original_ids[..m] {
        let expires: i64 = conn
            .query_row(
                "SELECT expires_at FROM events WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            expires > base_ts + 30 * 86400,
            "re-identified event {id} should have extended expires_at"
        );
    }
}
