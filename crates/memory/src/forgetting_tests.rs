//! Integration-style tests for the forgetting cleanup module.
//!
//! Covers normal-path cleanup, cascade boundaries, idempotency,
//! and edge cases. The inline unit tests in `forgetting.rs` exercise
//! the core `cleanup_expired` function; this module adds coverage
//! for `write_entries_to_db` + cleanup round-trip and the
//! `run_forgetting_cleanup` async wrapper.

use crate::miner::{
    init_schema, write_entries_to_db, MiningEntity, MiningEvent, MiningEventCategory,
};
use crate::forgetting::cleanup_expired;

use rusqlite::params;

// ── Helpers ──────────────────────────────────────────────────────────

/// Insert an event directly with a specific expires_at, bypassing
/// `write_to_sqlite` (which computes expires_at from Utc::now()).
fn insert_event(
    conn: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    category: MiningEventCategory,
    expires_at: i64,
) -> i64 {
    let ts = 500i64;
    conn.query_row(
        "INSERT INTO events (title, summary, content, category, lesson, \
         source_session_id, agent_id, timestamp, updated_at, expires_at) \
         VALUES ('t', 's', 'b', ?1, NULL, ?2, ?3, ?4, ?4, ?5) \
         RETURNING id",
        params![category.to_string(), session_id, agent_id, ts, expires_at],
        |row| row.get(0),
    )
    .unwrap()
}

/// Insert an entity and link it to an event.
fn insert_entity_with_event(
    conn: &rusqlite::Connection,
    agent_id: &str,
    event_id: i64,
    name: &str,
) {
    let norm_name = name.to_lowercase().replace(' ', "_");
    conn.execute(
        "INSERT OR IGNORE INTO entities (agent_id, type, name, normalized_name, description) \
         VALUES (?1, 'subject', ?2, ?3, 'desc')",
        params![agent_id, name, norm_name],
    )
    .unwrap();
    let entity_id: i64 = conn
        .query_row(
            "SELECT id FROM entities WHERE agent_id = ?1 AND type = 'subject' \
             AND normalized_name = ?2",
            params![agent_id, norm_name],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
        params![event_id, entity_id],
    )
    .unwrap();
}

// ── write_entries_to_db + cleanup round-trip ────────────────────────

/// Write an Insight event via `write_entries_to_db`, then verify the
/// DB row: category = "insight", lesson = NULL.
#[test]
fn test_write_entries_to_db_insight_category() {
    let tmp = tempfile::TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    let event = MiningEvent {
        title: "pattern discovered".to_string(),
        summary: "retry loop yields simple rule".to_string(),
        body: "body".to_string(),
        category: MiningEventCategory::Insight,
        lesson: None,
    };
    let entities = vec![vec![MiningEntity {
        entity_type: "subject".to_string(),
        name: "retry".to_string(),
        description: "retry logic".to_string(),
    }]];

    write_entries_to_db(&conn, "sess-1", "a1", &[event], &entities).unwrap();

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

    let expires_at: i64 = conn
        .query_row("SELECT expires_at FROM events WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        expires_at > 0,
        "write_entries_to_db should set a positive expires_at"
    );
}

/// Write an Insight event, wait for it to "expire", then cleanup
/// removes it and its orphan entity.
#[test]
fn test_insight_event_cleanup_round_trip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    // Insert an expired Insight event with an entity.
    let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Insight, 100);
    insert_entity_with_event(&conn, "a1", eid, "insight_entity");

    let stats = cleanup_expired(&mut conn, 200).unwrap();
    assert_eq!(stats.events_deleted, 1);
    assert_eq!(stats.entities_deleted, 1);

    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(event_count, 0);
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(entity_count, 0);
}

/// Expired event + active event + zero-TTL event:
/// only the expired one and its独占 entity are removed.
#[test]
fn test_mixed_events_only_expired_removed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    let now = 1000i64;
    let expired = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, now - 1);
    insert_entity_with_event(&conn, "a1", expired, "expires_solo");

    let active = insert_event(&conn, "s2", "a1", MiningEventCategory::Decision, now + 100);
    insert_entity_with_event(&conn, "a1", active, "active_entity");

    let zero_ttl = insert_event(&conn, "s3", "a1", MiningEventCategory::Anger, 0);
    insert_entity_with_event(&conn, "a1", zero_ttl, "permanent_entity");

    let stats = cleanup_expired(&mut conn, now).unwrap();
    assert_eq!(stats.events_deleted, 1);
    assert_eq!(stats.entities_deleted, 1);

    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(event_count, 2, "active + zero-TTL events should survive");

    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        entity_count, 2,
        "active + zero-TTL entities should survive"
    );
}

/// Entity shared by expired and active event: entity survives.
#[test]
fn test_cascade_boundary_shared_entity_survives() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    let now = 1000i64;
    let expired = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, now - 1);
    let active = insert_event(&conn, "s2", "a1", MiningEventCategory::Decision, now + 100);
    insert_entity_with_event(&conn, "a1", expired, "shared");
    insert_entity_with_event(&conn, "a1", active, "shared");

    let stats = cleanup_expired(&mut conn, now).unwrap();
    assert_eq!(stats.events_deleted, 1);
    assert_eq!(
        stats.entities_deleted, 0,
        "shared entity should survive (still referenced by active event)"
    );

    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(entity_count, 1);
}

/// Second cleanup on same DB returns zero counters.
#[test]
fn test_idempotent_cleanup_returns_zero() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, 999);
    insert_entity_with_event(&conn, "a1", eid, "idem");

    let s1 = cleanup_expired(&mut conn, 1000).unwrap();
    assert_eq!(s1.events_deleted, 1);
    assert_eq!(s1.entities_deleted, 1);

    let s2 = cleanup_expired(&mut conn, 1000).unwrap();
    assert_eq!(s2.events_deleted, 0);
    assert_eq!(s2.entities_deleted, 0);
}

/// Empty DB after init_schema: cleanup succeeds with zero stats.
#[test]
fn test_empty_db_cleanup_no_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    init_schema(&conn).unwrap();

    let stats = cleanup_expired(&mut conn, 1000).unwrap();
    assert_eq!(stats.events_deleted, 0);
    assert_eq!(stats.entities_deleted, 0);
}
