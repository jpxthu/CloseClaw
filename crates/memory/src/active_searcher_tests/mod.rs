//! Unit tests for active-searcher module.
//!
//! Covers SQLite schema, search logic, event association, dedup,
//! summarise, memory_injection slot, role exclusion, and timeout.

mod config_boundary_tests;
mod event_tests;
mod llm_config_tests;
mod schema_search_tests;

use std::path::Path;

use rusqlite::{params, Connection};

// ── Shared helpers ──────────────────────────────────────────────────────

/// Create a temporary SQLite database with the standard schema.
pub(crate) fn create_test_db(dir: &Path) -> Connection {
    let db_path = dir.join("test.db");
    let conn = Connection::open(&db_path).unwrap();
    init_test_schema(&conn);
    conn
}

/// Initialize the test database with entity_types, entities, event_entities,
/// and an events table (not part of init_schema yet).
pub(crate) fn init_test_schema(conn: &Connection) {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entity_types (
            id INTEGER PRIMARY KEY,
            type TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            weight REAL NOT NULL DEFAULT 1.0,
            similarity_threshold REAL NOT NULL DEFAULT 0.80,
            is_default INTEGER NOT NULL DEFAULT 0,
            is_active INTEGER NOT NULL DEFAULT 1
        );

        INSERT OR IGNORE INTO entity_types (id, type, name, description, weight, similarity_threshold, is_default, is_active) VALUES
            (1,  'time',         '时间',     '时间点', 1.0, 0.90, 0, 1),
            (2,  'location',      '地点',     '地点',   1.0, 0.75, 0, 1),
            (3,  'person',        '人物',     '人物',   1.2, 0.80, 0, 1),
            (4,  'organization',  '组织',     '组织',   1.1, 0.80, 0, 1),
            (5,  'subject',       '主题',     '主题',   1.5, 0.78, 1, 1),
            (6,  'product',       '产品',     '产品',   1.1, 0.80, 0, 1),
            (7,  'metric',        '指标',     '指标',   1.2, 0.85, 0, 1),
            (8,  'action',        '动作',     '动作',   1.3, 0.78, 1, 1),
            (9,  'work',          '作品',     '作品',   1.0, 0.80, 0, 1),
            (10, 'group',         '群体',     '群体',   1.0, 0.78, 0, 1),
            (11, 'tags',          '标签',     '标签',   0.5, 0.70, 1, 1);

        CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            type TEXT NOT NULL,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            description TEXT,
            UNIQUE(agent_id, type, normalized_name)
        );

        CREATE INDEX IF NOT EXISTS idx_entities_agent_normalized
            ON entities(agent_id, normalized_name);

        CREATE TABLE IF NOT EXISTS event_entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            FOREIGN KEY (entity_id) REFERENCES entities(id)
        );

        CREATE INDEX IF NOT EXISTS idx_event_entities_entity_id
            ON event_entities(entity_id);

        CREATE INDEX IF NOT EXISTS idx_event_entities_event_id
            ON event_entities(event_id);

        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            source_session_id TEXT NOT NULL
        );
        "#,
    )
    .unwrap();
}

/// Insert an entity and return its ID.
pub(crate) fn insert_entity(
    conn: &Connection,
    agent_id: &str,
    entity_type: &str,
    name: &str,
    normalized_name: &str,
) -> i64 {
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name)
         VALUES (?1, ?2, ?3, ?4)",
        params![agent_id, entity_type, name, normalized_name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Insert an event and return its ID.
pub(crate) fn insert_event(
    conn: &Connection,
    content: &str,
    timestamp: i64,
    session_id: &str,
) -> i64 {
    conn.execute(
        "INSERT INTO events (content, timestamp, source_session_id)
         VALUES (?1, ?2, ?3)",
        params![content, timestamp, session_id],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Link an event to an entity.
pub(crate) fn link_event_entity(conn: &Connection, event_id: i64, entity_id: i64) {
    conn.execute(
        "INSERT INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
        params![event_id, entity_id],
    )
    .unwrap();
}
