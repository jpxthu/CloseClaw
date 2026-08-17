//! SQLite operations for the memory miner.
//!
//! Contains schema initialization, read/write helpers, and entity
//! catalog loading — everything that touches `rusqlite::Connection`
//! directly.  Extracted from `miner.rs` to keep that module focused
//! on the high-level mining orchestration.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::params;

use closeclaw_config::agents::{
    default_forgetting_initial_ttl_days, default_forgetting_reidentify_extension_days,
};

use super::{MinerError, MiningEntity, MiningEvent};

// ── WriteConfig ───────────────────────────────────────────────────────

/// Parameters for [`write_to_sqlite`], bundled to stay within the
/// 6-parameter function limit.
pub(crate) struct WriteConfig<'a> {
    /// Session ID these events belong to.
    pub session_id: &'a str,
    /// Agent that produced the events.
    pub agent_id: &'a str,
    /// Events to write.
    pub events: &'a [MiningEvent],
    /// Per-event entity lists (same length as `events`).
    pub entities: &'a [Vec<MiningEntity>],
    /// Initial TTL in days for new event `expires_at`.
    pub initial_ttl_days: i64,
    /// Days to extend `expires_at` on re-identified events.
    pub reidentify_extension_days: i64,
}

// ── Schema ────────────────────────────────────────────────────────────

/// Seed the 11 SAG entity types into `entity_types`.
///
/// Uses `INSERT OR IGNORE` so repeated calls are idempotent.
fn seed_entity_types(conn: &rusqlite::Connection) -> Result<(), MinerError> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO entity_types \
         (id, type, name, description, weight, \
          similarity_threshold, is_default, is_active) \
         VALUES \
         (1, 'time', '时间', \
          '时间点、时期、日期、年份等时间表达', \
          1.0, 0.90, 0, 1), \
         (2, 'location', '地点', \
          '国家、城市、地区、地点等物理位置', \
          1.0, 0.75, 0, 1), \
         (3, 'person', '人物', \
          '人物和具名个体（含 agent 角色、用户身份）', \
          1.2, 0.80, 0, 1), \
         (4, 'organization', '组织', \
          '公司、机构、团队等组织', \
          1.1, 0.80, 0, 1), \
         (5, 'subject', '主题', \
          '主要主题、概念和课题', \
          1.5, 0.78, 1, 1), \
         (6, 'product', '产品', \
          '产品、服务、项目和命名交付物', \
          1.1, 0.80, 0, 1), \
         (7, 'metric', '指标', \
          '数字、指标、度量、金额和统计数据', \
          1.2, 0.85, 0, 1), \
         (8, 'action', '动作', \
          '重要动作、变更、决策和操作', \
          1.3, 0.78, 1, 1), \
         (9, 'work', '作品', \
          '创作物、文档、论文、书籍、报告', \
          1.0, 0.80, 0, 1), \
         (10, 'group', '群体', \
          '群体、社区、受众和人口', \
          1.0, 0.78, 0, 1), \
         (11, 'tags', '标签', \
          '兜底标签，当无特定类型匹配时使用', \
          0.5, 0.70, 1, 1);",
    )
    .map_err(|e| MinerError::Sqlite(e.to_string()))
}

/// Create all mining tables (DDL only, no seed data).
fn create_tables(conn: &rusqlite::Connection) -> Result<(), MinerError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            content TEXT NOT NULL,
            category TEXT NOT NULL,
            lesson TEXT,
            source_session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL DEFAULT '',
            timestamp INTEGER NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT 0,
            expires_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            type TEXT NOT NULL,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            description TEXT DEFAULT '',
            UNIQUE(agent_id, type, normalized_name)
        );
        CREATE TABLE IF NOT EXISTS event_entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            FOREIGN KEY (event_id) REFERENCES events(id),
            FOREIGN KEY (entity_id) REFERENCES entities(id)
        );
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
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            mined INTEGER NOT NULL DEFAULT 0,
            mined_at INTEGER
        );",
    )
    .map_err(|e| MinerError::Sqlite(e.to_string()))
}

/// Add `expires_at` column to existing databases (idempotent).
fn migrate_add_expires_at(conn: &rusqlite::Connection) -> Result<(), MinerError> {
    match conn.execute(
        "ALTER TABLE events \
         ADD COLUMN expires_at INTEGER \
         NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(MinerError::Sqlite(msg));
            }
        }
    }
    Ok(())
}

/// Initialize the SQLite schema for mining tables.
///
/// Creates tables, seeds entity types, and runs migrations.
pub(crate) fn init_schema(conn: &rusqlite::Connection) -> Result<(), MinerError> {
    create_tables(conn)?;
    seed_entity_types(conn)?;
    migrate_add_expires_at(conn)
}

// ── Write helpers ─────────────────────────────────────────────────────

/// Extend `expires_at` on an existing event row (re-identify path).
///
/// Returns an error if the `UPDATE` matched zero rows, which indicates a
/// stale or invalid `existing_id`.
fn extend_reidentified_event(
    conn: &rusqlite::Connection,
    existing_id: i64,
    extension_days: i64,
) -> Result<(), MinerError> {
    let now = Utc::now().timestamp();
    let extension = extension_days * 86400;
    let changed = conn
        .execute(
            "UPDATE events SET expires_at = MAX(expires_at, ?1) + ?2 WHERE id = ?3",
            params![now, extension, existing_id],
        )
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    if changed == 0 {
        return Err(MinerError::Sqlite(format!(
            "re-identify UPDATE matched 0 rows for event id={existing_id}"
        )));
    }
    Ok(())
}

/// Upsert entities and link them to a newly inserted event.
fn assign_entities_to_event(
    conn: &rusqlite::Connection,
    agent_id: &str,
    event_id: i64,
    entities: &[MiningEntity],
) -> Result<(), MinerError> {
    for entity in entities {
        let norm_name = normalize_entity_name(&entity.name);
        conn.execute(
            "INSERT OR IGNORE INTO entities (agent_id, type, name, normalized_name, description)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent_id,
                entity.entity_type,
                entity.name,
                norm_name,
                entity.description
            ],
        )
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
        let entity_id: i64 = conn
            .query_row(
                "SELECT id FROM entities
                 WHERE agent_id = ?1
                 AND type = ?2
                 AND normalized_name = ?3",
                params![agent_id, entity.entity_type, norm_name],
                |row| row.get(0),
            )
            .map_err(|e| MinerError::Sqlite(e.to_string()))?;
        conn.execute(
            "INSERT OR IGNORE INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
            params![event_id, entity_id],
        )
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    }
    Ok(())
}

/// Write events and entities to SQLite.
///
/// When `reidentified_event_id` is `Some(id)`, the event is treated as
/// a re-occurrence: `expires_at` on the existing row is extended by
/// `reidentify_extension_days` and no new row is inserted.  Otherwise
/// the event is inserted normally.
pub(crate) fn write_to_sqlite(
    conn: &rusqlite::Connection,
    cfg: &WriteConfig<'_>,
) -> Result<(), MinerError> {
    let now = Utc::now().timestamp();
    for (event, event_entities) in cfg.events.iter().zip(cfg.entities.iter()) {
        if let Some(existing_id) = event.reidentified_event_id {
            extend_reidentified_event(conn, existing_id, cfg.reidentify_extension_days)?;
            continue;
        }
        let expires_at = now + cfg.initial_ttl_days * 86400;
        let event_id: i64 = conn
            .query_row(
                "INSERT INTO events \
                 (title, summary, content, category, lesson, \
                  source_session_id, agent_id, timestamp, \
                  updated_at, expires_at) \
                 VALUES \
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9) \
                 RETURNING id",
                params![
                    event.title,
                    event.summary,
                    event.body,
                    event.category.to_string(),
                    event.lesson,
                    cfg.session_id,
                    cfg.agent_id,
                    now,
                    expires_at,
                ],
                |row| row.get(0),
            )
            .map_err(|e| MinerError::Sqlite(e.to_string()))?;
        assign_entities_to_event(conn, cfg.agent_id, event_id, event_entities)?;
    }
    conn.execute(
        "INSERT INTO sessions (id, mined, mined_at) \
         VALUES (?1, 1, ?2) \
         ON CONFLICT(id) \
         DO UPDATE SET mined = 1, mined_at = ?2",
        params![cfg.session_id, now],
    )
    .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    Ok(())
}

/// Write entries to SQLite (public interface).
//
// NOTE: uses default initial_ttl_days; prefer write_to_sqlite for configured TTL.
#[allow(dead_code)]
pub fn write_entries_to_db(
    conn: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    events: &[MiningEvent],
    entities: &[Vec<MiningEntity>],
) -> Result<(), MinerError> {
    init_schema(conn)?;
    write_to_sqlite(
        conn,
        &WriteConfig {
            session_id,
            agent_id,
            events,
            entities,
            initial_ttl_days: default_forgetting_initial_ttl_days(),
            reidentify_extension_days: default_forgetting_reidentify_extension_days(),
        },
    )
}

// ── Read helpers ──────────────────────────────────────────────────────

/// Load recent events within the dedup window for Miner 1 context.
///
/// Returns a tuple of `(formatted_text, event_ids)` where the formatted
/// text includes `[{id}]` prefixes so the LLM can reference specific
/// events for re-identification. The `event_ids` vector contains the
/// corresponding database IDs in the same order.
pub(crate) fn load_recent_events(
    conn: &rusqlite::Connection,
    exclude_session: &str,
    agent_id: &str,
    dedup_window_days: i32,
) -> Result<(String, Vec<i64>), MinerError> {
    let cutoff = Utc::now().timestamp() - (dedup_window_days as i64 * 86400);
    let sql = "SELECT id, title, summary, category FROM events
               WHERE source_session_id != ?1 AND timestamp >= ?2 AND agent_id = ?3
               ORDER BY timestamp DESC LIMIT 20";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let rows: Vec<(i64, String, String, String)> = stmt
        .query_map(params![exclude_session, cutoff, agent_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let ids: Vec<i64> = rows.iter().map(|(id, _, _, _)| *id).collect();
    let text = rows
        .iter()
        .map(|(id, title, summary, category)| format!("- [{id}] [{category}] {title}: {summary}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, ids))
}

/// Build catalog sections from type definitions and entities.
///
/// Returns a list of sections, one per entity type. Each section
/// starts with a type header (`## type (name): description`),
/// followed by entity lines (`- entity_name: description`).
fn build_catalog_sections(
    types: &[(String, String, String)],
    entities_by_type: &HashMap<String, Vec<(String, String)>>,
) -> Vec<String> {
    let mut extra_types: std::collections::HashSet<String> =
        entities_by_type.keys().cloned().collect();
    let mut sections = Vec::new();
    for (typ, name, desc) in types {
        let mut section = format!("## {typ} ({name}): {desc}");
        if let Some(type_entities) = entities_by_type.get(typ) {
            for (entity_name, entity_desc) in type_entities {
                section.push_str(&format!("\n- {entity_name}: {entity_desc}"));
            }
        }
        sections.push(section);
        extra_types.remove(typ);
    }
    let mut remaining: Vec<_> = extra_types.into_iter().collect();
    remaining.sort();
    for typ in remaining {
        if let Some(type_entities) = entities_by_type.get(&typ) {
            let mut section = format!("## {typ}");
            for (entity_name, entity_desc) in type_entities {
                section.push_str(&format!("\n- {entity_name}: {entity_desc}"));
            }
            sections.push(section);
        }
    }
    sections
}

/// Load entity/type catalog from SQLite, sorted by type → normalized_name.
///
/// Merges `entity_types` table (type definitions) with `entities` table
/// (existing entities). Output groups by type: each section starts with
/// a type header (`## type (name): description`), followed by entity lines
/// (`- entity_name: description`). All 11 types are always listed even
/// when no entities exist for that type.
pub(crate) fn load_entity_catalog(
    conn: &rusqlite::Connection,
    agent_id: &str,
) -> Result<String, MinerError> {
    let type_sql = "SELECT type, name, description \
         FROM entity_types WHERE is_active = 1 \
         ORDER BY type";
    let mut type_stmt = conn
        .prepare(type_sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let types: Vec<(String, String, String)> = type_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    let entity_sql = "SELECT type, name, description \
         FROM entities WHERE agent_id = ?1 \
         ORDER BY type, normalized_name";
    let mut entity_stmt = conn
        .prepare(entity_sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let entities: Vec<(String, String, String)> = entity_stmt
        .query_map(params![agent_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    let mut entities_by_type: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (typ, name, desc) in entities {
        entities_by_type.entry(typ).or_default().push((name, desc));
    }
    let sections = build_catalog_sections(&types, &entities_by_type);
    Ok(sections.join("\n\n"))
}

/// Load entity type → similarity_threshold mapping from SQLite.
///
/// Returns a HashMap where keys are entity type names (e.g. "time",
/// "subject") and values are the similarity_threshold thresholds.
pub(crate) fn load_entity_type_thresholds(
    conn: &rusqlite::Connection,
) -> Result<HashMap<String, f64>, MinerError> {
    let sql = "SELECT type, similarity_threshold FROM entity_types WHERE is_active = 1";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let mut map = HashMap::new();
    for row in rows {
        let (typ, threshold) = row.map_err(|e| MinerError::Sqlite(e.to_string()))?;
        map.insert(typ, threshold);
    }
    Ok(map)
}

// ── Utilities ─────────────────────────────────────────────────────────

/// Normalize an entity name: lowercase, replace spaces with underscores.
pub(crate) fn normalize_entity_name(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}
