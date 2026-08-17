use crate::miner::load_existing_entities_by_type;

use rusqlite::Connection;
use tempfile::TempDir;

// ── load_existing_entities_by_type tests ──────────────────────────────

/// Multiple entity types should be grouped correctly by type.
#[test]
fn test_load_existing_entities_by_type_groups_by_type() {
    let tmp = TempDir::new().unwrap();
    let conn = Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'Rust', 'rust', 'a language')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'subject', 'Python', 'python', 'another language')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'person', 'Alice', 'alice', 'a person')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a1', 'action', 'Build', 'build', 'an action')",
        [],
    )
    .unwrap();
    // Different agent should not appear.
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('a2', 'subject', 'Java', 'java', 'other agent')",
        [],
    )
    .unwrap();

    let map = load_existing_entities_by_type(&conn, "a1").unwrap();

    assert_eq!(map.len(), 3, "should have 3 types: subject, person, action");
    assert_eq!(
        map.get("subject").unwrap().len(),
        2,
        "subject should have 2 entities"
    );
    assert_eq!(
        map.get("person").unwrap().len(),
        1,
        "person should have 1 entity"
    );
    assert_eq!(
        map.get("action").unwrap().len(),
        1,
        "action should have 1 entity"
    );

    // Verify tuple contents for one entity.
    let subject_entities = map.get("subject").unwrap();
    // Entities are ordered by normalized_name.
    assert_eq!(subject_entities[0].1, "Python", "Python comes before Rust");
    assert_eq!(subject_entities[0].3, "python");
    assert_eq!(subject_entities[1].1, "Rust");
    assert_eq!(subject_entities[1].3, "rust");

    // a2's Java should not appear.
    assert!(!map.contains_key("java"));
}

/// Agent with no entities should return an empty HashMap.
#[test]
fn test_load_existing_entities_by_type_empty_agent() {
    let tmp = TempDir::new().unwrap();
    let conn = Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    // Insert entities for a different agent.
    conn.execute(
        "INSERT INTO entities (agent_id, type, name, normalized_name, description)
         VALUES ('other', 'subject', 'Something', 'something', 'not mine')",
        [],
    )
    .unwrap();

    let map = load_existing_entities_by_type(&conn, "a1").unwrap();
    assert!(
        map.is_empty(),
        "agent with no entities should return empty HashMap"
    );
}

/// Empty DB (no entities at all) should return an empty HashMap.
#[test]
fn test_load_existing_entities_by_type_empty_db() {
    let tmp = TempDir::new().unwrap();
    let conn = Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();

    let map = load_existing_entities_by_type(&conn, "a1").unwrap();
    assert!(map.is_empty(), "empty DB should return empty HashMap");
}
