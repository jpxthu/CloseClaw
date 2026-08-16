//! Design-alignment tests for dreaming scoring and entity identification.
//!
//! Verifies the two core fixes from the dreaming design-doc alignment:
//! 1. Type weight is a multiplicative factor (base × type_weight), not an
//!    additive dimension in the scoring formula.
//! 2. Entity grouping and cross-agent detection use `normalized_name`
//!    instead of raw `name`.

use crate::dreaming::{DreamingPipeline, EntityGroup, EntryCategory, MemoryEntry};
use crate::miner::init_schema;
use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingConfig, DreamingScoringConfig, DreamingThresholdConfig,
};
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_entry(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    entity_type: &str,
    entity_name: &str,
    minutes_ago: i64,
) -> MemoryEntry {
    let timestamp = chrono::Utc::now() - chrono::Duration::minutes(minutes_ago);
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp,
        source_session_id: session_id.to_string(),
        lesson: None,
        tags: Vec::new(),
        score: 0.0,
        event_id: 0,
        entity_type: entity_type.to_string(),
        entity_name: entity_name.to_string(),
        updated_at: timestamp,
    }
}

fn make_pipeline_with_db(db_path: &std::path::Path) -> DreamingPipeline {
    DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(-100.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    })
    .with_db_path(db_path)
}

/// Create a DB with full schema and entity_types seed data.
fn make_db() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    init_schema(&conn).unwrap();
    (tmp, db_path)
}

// ── 1. Multiplier semantics ──────────────────────────────────────────────

/// subject (weight 1.5) scores ~1.5× person (weight 1.2), confirming
/// type_weight is applied as a final multiplier on the base score.
#[test]
fn test_multiplier_semantics() {
    let (_tmp, db_path) = make_db();
    let pipeline = make_pipeline_with_db(&db_path);

    let mut subject_entry = make_entry(EntryCategory::Error, "err1", "s1", "subject", "deploy", 10);
    subject_entry.event_id = 1;
    let subject_group = EntityGroup {
        entity_name: "deploy".into(),
        entity_type: "subject".into(),
        entries: vec![subject_entry],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    };

    let mut person_entry = make_entry(EntryCategory::Error, "err2", "s1", "person", "alice", 10);
    person_entry.event_id = 2;
    let person_group = EntityGroup {
        entity_name: "alice".into(),
        entity_type: "person".into(),
        entries: vec![person_entry],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    };

    let deep = pipeline.deep_stage(vec![subject_group, person_group]);
    let subject = deep.iter().find(|g| g.entity_type == "subject").unwrap();
    let person = deep.iter().find(|g| g.entity_type == "person").unwrap();

    // base is identical (same frequency, recency, explicitness, cross_agent,
    // negative_signal).  Ratio must match weight ratio 1.5/1.2 = 1.25.
    let ratio = subject.score / person.score;
    assert!(
        (ratio - 1.25).abs() < 0.01,
        "subject/person score ratio should be ~1.25, got {ratio}"
    );
}

// ── 2. Boundary fallback ─────────────────────────────────────────────────

/// entity_types table missing the requested type → fallback weight 1.0.
#[test]
fn test_boundary_fallback_unknown_type() {
    let (_tmp, db_path) = make_db();
    let pipeline = make_pipeline_with_db(&db_path);

    let entry = make_entry(
        EntryCategory::Error,
        "err1",
        "s1",
        "nonexistent",
        "ghost",
        10,
    );
    let group = EntityGroup {
        entity_name: "ghost".into(),
        entity_type: "nonexistent".into(),
        entries: vec![entry],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    };

    let deep = pipeline.deep_stage(vec![group]);
    let score = deep[0].score;
    // base = 1.0 (frequency=1, all other dims=0), weight fallback = 1.0
    assert!(
        (score - 1.0).abs() < 0.001,
        "unknown type weight should fallback to 1.0, got {score}"
    );
}

/// is_active=0 type → fallback weight 1.0.
#[test]
fn test_boundary_fallback_inactive_type() {
    let (_tmp, db_path) = make_db();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE entity_types SET is_active = 0 WHERE type = 'person'",
        [],
    )
    .unwrap();

    let pipeline = make_pipeline_with_db(&db_path);
    let entry = make_entry(EntryCategory::Error, "err1", "s1", "person", "alice", 10);
    let group = EntityGroup {
        entity_name: "alice".into(),
        entity_type: "person".into(),
        entries: vec![entry],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    };

    let deep = pipeline.deep_stage(vec![group]);
    let score = deep[0].score;
    assert!(
        (score - 1.0).abs() < 0.001,
        "inactive type weight should fallback to 1.0, got {score}"
    );
}

// ── 3. Negative base consistency ─────────────────────────────────────────

/// With negative_signal_weight < 0, groups containing category reversals
/// score lower.  The type weight multiplier acts on the post-deduction base,
/// preserving the relative order imposed by the scoring formula.
#[test]
fn test_negative_base_consistency() {
    let (_tmp, db_path) = make_db();

    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(2.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(-3.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(-100.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    })
    .with_db_path(&db_path);

    // Same-category group (no negative signal): negative_signal = 0.
    let mut e1 = make_entry(EntryCategory::Error, "err1", "s1", "subject", "same", 10);
    e1.event_id = 1;
    let mut e2 = make_entry(EntryCategory::Error, "err2", "s1", "subject", "same", 5);
    e2.event_id = 2;

    // Mixed-category group (reversal → negative_signal > 0):
    // first entry is Error, second is Decision → 1 reversal out of 2 → 0.5.
    let mut e3 = make_entry(EntryCategory::Error, "err3", "s2", "subject", "mixed", 10);
    e3.event_id = 3;
    let mut e4 = make_entry(EntryCategory::Decision, "dec1", "s2", "subject", "mixed", 5);
    e4.event_id = 4;

    let groups = vec![
        EntityGroup {
            entity_name: "same".into(),
            entity_type: "subject".into(),
            entries: vec![e1, e2],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "mixed".into(),
            entity_type: "subject".into(),
            entries: vec![e3, e4],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
    ];

    let deep = pipeline.deep_stage(groups);
    let same = deep.iter().find(|g| g.entity_name == "same").unwrap();
    let mixed = deep.iter().find(|g| g.entity_name == "mixed").unwrap();

    // same: base = 2.0×1 = 2.0, negative_signal = 0 → final = 2.0 × 1.5 = 3.0
    // mixed: base = 2.0×1 + (-3.0)×0.5 = 2.0−1.5 = 0.5, final = 0.5 × 1.5 = 0.75
    assert!(
        same.score > mixed.score,
        "same-category ({}) should score higher than mixed ({})",
        same.score,
        mixed.score
    );
    assert!(
        (same.score - 3.0).abs() < 0.01,
        "same-group score should be ~3.0, got {}",
        same.score
    );
    assert!(
        (mixed.score - 0.75).abs() < 0.01,
        "mixed-group score should be ~0.75, got {}",
        mixed.score
    );
}

// ── 4. normalized_name clustering ────────────────────────────────────────

/// Entities with raw names "Banana" and "banana" but identical
/// normalized_name + type are grouped together (frequency merged).
#[test]
fn test_normalized_name_clustering() {
    let (_tmp, db_path) = make_db();

    // Insert two entities with different raw names but same normalized_name.
    // Use different agent_ids because the UNIQUE constraint is on
    // (agent_id, type, normalized_name).
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // Session must exist with mined=1 for load_entries_from_sqlite.
        conn.execute("INSERT INTO sessions (id, mined) VALUES ('s1', 1)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-a', 'product', 'Banana', 'banana')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-b', 'product', 'banana', 'banana')",
            [],
        )
        .unwrap();
        // Two events linked to each entity.
        conn.execute(
            "INSERT INTO events (title, summary, content, category, lesson,
             source_session_id, agent_id, timestamp, updated_at) VALUES \
             ('title1', 'sum1', 'err1', 'error', NULL, 's1', 'agent-a', \
             1700000000, 1700000000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (title, summary, content, category, lesson,
             source_session_id, agent_id, timestamp, updated_at) VALUES \
             ('title2', 'sum2', 'err2', 'error', NULL, 's1', 'agent-b', \
             1700000060, 1700000060)",
            [],
        )
        .unwrap();
        // Link event 1 → entity 1, event 2 → entity 2.
        conn.execute(
            "INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO event_entities (event_id, entity_id) VALUES (2, 2)",
            [],
        )
        .unwrap();
    }

    let pipeline = make_pipeline_with_db(&db_path);
    let entries = pipeline
        .load_entries_from_sqlite(&rusqlite::Connection::open(&db_path).unwrap(), "s1")
        .unwrap();

    // Both entries should have entity_name = "banana" (normalized).
    assert_eq!(entries.len(), 2, "should load 2 entries");
    for e in &entries {
        assert_eq!(
            e.entity_name, "banana",
            "entity_name should be normalized, got {:?}",
            e.entity_name
        );
    }

    // REM stage: both entries should merge into one EntityGroup.
    let chunks = vec![entries];
    let groups = pipeline.rem_stage(chunks);
    assert_eq!(groups.len(), 1, "should produce 1 entity group");
    // Frequency counts distinct sessions; both entries are from s1 → 1.
    assert_eq!(
        groups[0].frequency, 1,
        "frequency should count distinct sessions, got {}",
        groups[0].frequency
    );
    assert_eq!(
        groups[0].entries.len(),
        2,
        "group should contain both entries"
    );
}

// ── 5. normalized_name cross-agent ──────────────────────────────────────

/// Agent A holds "Banana" and agent B holds "banana" (same type + same
/// normalized_name).  rem_stage merges them into a single EntityGroup
/// and cross_agent_count ≥ 2 (both agents present in the entity table).
#[test]
fn test_normalized_name_cross_agent() {
    let (_tmp, db_path) = make_db();
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // Agent A: name "Banana" → normalized "banana".
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-a', 'product', 'Banana', 'banana')",
            [],
        )
        .unwrap();
        // Agent B: name "banana" → normalized "banana".
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-b', 'product', 'banana', 'banana')",
            [],
        )
        .unwrap();
    }

    let pipeline = make_pipeline_with_db(&db_path);

    // Both entries share normalized_name "banana" and type "product".
    let e1 = make_entry(
        EntryCategory::Error,
        "err from agent-a",
        "s1",
        "product",
        "banana",
        10,
    );
    let e2 = make_entry(
        EntryCategory::Error,
        "err from agent-b",
        "s2",
        "product",
        "banana",
        5,
    );

    // REM stage: entries from different sessions with same normalized_name
    // and type should merge into one group.
    let groups = pipeline.rem_stage(vec![vec![e1, e2]]);
    let banana = groups
        .iter()
        .find(|g| g.entity_name == "banana")
        .expect("should find banana group");

    // cross_agent_count reflects distinct agents in the entities table;
    // both agent-a and agent-b hold this entity → count ≥ 2.
    assert!(
        banana.cross_agent_count >= 2,
        "cross_agent_count should be ≥ 2, got {}",
        banana.cross_agent_count
    );
}
