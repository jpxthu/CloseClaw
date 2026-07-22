use crate::active_searcher::{ActiveSearcher, ActiveSearcherConfig};
use crate::active_searcher_llm::should_trigger_role;

use super::{create_test_db, insert_entity};

// ── SQLite schema tests ──────────────────────────────────────────────────

#[test]
fn test_sqlite_schema_tables_created() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());

    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert!(tables.contains(&"entity_types".to_string()));
    assert!(tables.contains(&"entities".to_string()));
    assert!(tables.contains(&"event_entities".to_string()));
}

#[test]
fn test_sqlite_schema_seed_data_integrity() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entity_types", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 11, "should have 11 seed entity types");

    let types: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT type FROM entity_types ORDER BY id")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert_eq!(types[0], "time");
    assert_eq!(types[2], "person");
    assert_eq!(types[4], "subject");
}

#[test]
fn test_sqlite_schema_unique_constraint() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());

    insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let result = conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["agent-1", "person", "Alice Alt", "alice"],
    );
    assert!(
        result.is_err(),
        "UNIQUE constraint should prevent duplicate"
    );
}

// ── Search logic tests ───────────────────────────────────────────────────

#[test]
fn test_search_entities_exact_match() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "person", "Alice", "alice");

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let results = searcher
        .search_entities("agent-1", &["alice".into()])
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Alice");
    assert_eq!(results[0].normalized_name, "alice");
}

#[test]
fn test_search_entities_fuzzy_match() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "person", "Alice Smith", "alice smith");

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let results = searcher
        .search_entities("agent-1", &["alice".into()])
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Alice Smith");
}

#[test]
fn test_search_entities_agent_id_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    insert_entity(&conn, "agent-2", "person", "Bob", "bob");

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let r1 = searcher
        .search_entities("agent-1", &["alice".into()])
        .unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].agent_id, "agent-1");

    let r1_all = searcher
        .search_entities("agent-1", &["bob".into()])
        .unwrap();
    assert!(r1_all.is_empty(), "agent-1 should not see Bob");

    let r2 = searcher
        .search_entities("agent-2", &["bob".into()])
        .unwrap();
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0].agent_id, "agent-2");
}

#[test]
fn test_search_entities_type_weight_ordering() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // "subject" has weight 1.5, "person" has weight 1.2, "tags" has weight 0.5
    insert_entity(&conn, "agent-1", "tags", "alice", "alice");
    insert_entity(&conn, "agent-1", "subject", "alice topic", "alice topic");
    insert_entity(&conn, "agent-1", "person", "alice person", "alice person");

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let results = searcher
        .search_entities("agent-1", &["alice".into()])
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].entity_type, "subject");
    assert_eq!(results[1].entity_type, "person");
    assert_eq!(results[2].entity_type, "tags");
}

#[test]
fn test_search_entities_no_data_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let results = searcher
        .search_entities("agent-1", &["anything".into()])
        .unwrap();
    assert!(results.is_empty());
}

// ── Role exclusion tests ─────────────────────────────────────────────────

#[test]
fn test_role_exclusion_and_trigger() {
    assert!(!should_trigger_role("memory-miner"));
    assert!(!should_trigger_role("dreaming"));
    assert!(should_trigger_role("user"));
    assert!(should_trigger_role("assistant"));
    assert!(!ActiveSearcher::should_trigger("memory-miner"));
    assert!(ActiveSearcher::should_trigger("user"));
}

// ── is_active / is_default type tests ────────────────────────────────────

/// is_active=0 entity types are excluded from search results.
#[test]
fn test_active_searcher_excludes_inactive_types() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    conn.execute(
        "UPDATE entity_types SET is_active = 0 WHERE type = 'person'",
        [],
    )
    .unwrap();
    insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["alice".into()])
        .unwrap();
    assert!(results.is_empty());
}

/// is_default=1 types rank first among same-weight types.
#[test]
fn test_active_searcher_default_type_priority() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    conn.execute(
        "INSERT INTO entity_types (type, name, description, weight, is_default, is_active)
         VALUES ('alpha', 'Alpha', 'alpha type', 2.0, 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entity_types (type, name, description, weight, is_default, is_active)
         VALUES ('beta', 'Beta', 'beta type', 2.0, 0, 1)",
        [],
    )
    .unwrap();
    insert_entity(&conn, "agent-1", "beta", "shared name", "shared name");
    insert_entity(&conn, "agent-1", "alpha", "shared name a", "shared name a");
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["shared name".into()])
        .unwrap();
    assert_eq!(results.len(), 2, "both entities should match");
    assert_eq!(results[0].entity_type, "alpha");
    assert_eq!(results[1].entity_type, "beta");
}
