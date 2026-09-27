//! SQLite schema initialization for the session storage backend.
//!
//! DDL and column migrations live here to keep `sqlite.rs` focused on the
//! storage API surface.

use crate::persistence::PersistenceError;
use rusqlite::Connection;

/// Add a column to sessions table if it doesn't already exist.
///
/// SAFETY: `column` and `col_type` are always hardcoded string literals
/// passed from `init_schema`, never from user input. This eliminates
/// SQL injection risk in the format-string `ALTER TABLE` statement.
fn add_column_if_not_exists(
    conn: &Connection,
    column: &str,
    col_type: &str,
) -> Result<(), PersistenceError> {
    let exists = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        cols.iter().any(|name| name == column)
    };
    if !exists {
        let sql = format!("ALTER TABLE sessions ADD COLUMN {column} {col_type}");
        conn.execute(&sql, [])
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    }
    Ok(())
}

/// Initialize the database schema
pub(super) fn init_schema(conn: &Connection) -> Result<(), PersistenceError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            role TEXT NOT NULL,
            channel TEXT NOT NULL,
            chat_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            title TEXT,
            last_message_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            archived_at INTEGER,
            message_count INTEGER DEFAULT 0,
            metadata TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_status
            ON sessions(status);

        CREATE INDEX IF NOT EXISTS idx_sessions_last_message_at
            ON sessions(last_message_at);

        CREATE INDEX IF NOT EXISTS idx_sessions_agent_id
            ON sessions(agent_id);

        CREATE INDEX IF NOT EXISTS idx_sessions_archived_at
            ON sessions(archived_at)
            WHERE archived_at IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_sessions_agent_role
            ON sessions(agent_id, role);

        -- Entity types table (seed data for 11 SAG entity types)
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
            (1,  'time',         '时间',     '时间点、时期、日期、年份等时间表达', 1.0, 0.90, 0, 1),
            (2,  'location',      '地点',     '国家、城市、地区、地点等物理位置', 1.0, 0.75, 0, 1),
            (3,  'person',        '人物',     '人物和具名个体（含 agent 角色、用户身份）', 1.2, 0.80, 0, 1),
            (4,  'organization',  '组织',     '公司、机构、团队等组织', 1.1, 0.80, 0, 1),
            (5,  'subject',       '主题',     '主要主题、概念和课题', 1.5, 0.78, 1, 1),
            (6,  'product',       '产品',     '产品、服务、项目和命名交付物', 1.1, 0.80, 0, 1),
            (7,  'metric',        '指标',     '数字、指标、度量、金额和统计数据', 1.2, 0.85, 0, 1),
            (8,  'action',        '动作',     '重要动作、变更、决策和操作', 1.3, 0.78, 1, 1),
            (9,  'work',          '作品',     '创作物、文档、论文、书籍、报告', 1.0, 0.80, 0, 1),
            (10, 'group',         '群体',     '群体、社区、受众和人口', 1.0, 0.78, 0, 1),
            (11, 'tags',          '标签',     '兜底标签，当无特定类型匹配时使用', 0.5, 0.70, 1, 1);

        -- Entities table (per-agent entity storage)
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

        -- Event-entity association table
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
        "#,
    )
    .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    // Snapshot metadata table — independent from SessionCheckpoint.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS snapshot_metas (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            status TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_snapshot_metas_session_id
            ON snapshot_metas(session_id);
        "#,
    )
    .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    for (col, col_type) in [
        ("thread_id", "TEXT"),
        ("sender_id", "TEXT"),
        ("platform", "TEXT"),
        ("peer_id", "TEXT"),
        ("account_id", "TEXT"),
        ("parent_session_id", "TEXT"),
        ("depth", "TEXT"),
        ("mined", "TEXT"),
        ("dreaming_status", "TEXT"),
        ("plan_state", "TEXT"),
        ("mined_at", "INTEGER"),
        ("last_user_activity_at", "INTEGER"),
    ] {
        add_column_if_not_exists(conn, col, col_type)?;
    }

    Ok(())
}
