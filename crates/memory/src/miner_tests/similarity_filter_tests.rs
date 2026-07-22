use crate::embedding::{cosine_similarity, EntityEmbedder, NgramEmbedder};
use crate::miner::{load_entity_type_thresholds, MemoryMiner, MinerConfig, MiningEntity};
use crate::miner_llm::MockMinerLlmCaller;
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::TranscriptCleanRules;
use closeclaw_session::persistence::SessionCheckpoint;

use tempfile::TempDir;

use super::{make_entity, make_event};

/// Lenient rules: 1 turn, 1 owner message, md format.
fn lenient_rules() -> TranscriptCleanRules {
    TranscriptCleanRules {
        min_turns: Some(1),
        min_owner_msgs: Some(1),
        format: Some("md".to_string()),
    }
}

// ── Similarity threshold load tests ──────────────────────────────────

#[test]
fn test_load_entity_type_thresholds_all_types() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    let thresholds = load_entity_type_thresholds(&conn).unwrap();
    assert_eq!(thresholds.len(), 11);
    assert_eq!(thresholds["time"], 0.90);
    assert_eq!(thresholds["tags"], 0.70);
}

#[test]
fn test_load_entity_type_thresholds_excludes_inactive() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    conn.execute(
        "UPDATE entity_types SET is_active = 0 WHERE type = 'time'",
        [],
    )
    .unwrap();
    let thresholds = load_entity_type_thresholds(&conn).unwrap();
    assert!(!thresholds.contains_key("time"));
    assert!(thresholds.contains_key("tags"));
}

// ── Miner session similarity filter tests ────────────────────────────

/// High-threshold entity filtered when loosely related to event.
#[tokio::test]
async fn test_mine_session_filters_high_threshold_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-ht".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("January 2025", "time")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("ht.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-ht", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        0,
        "high-threshold time entity should be filtered"
    );
}

/// Low-threshold entity retained when similar to event.
#[tokio::test]
async fn test_mine_session_keeps_low_threshold_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-lt".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Rust language", "tags")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("lt.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-lt", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "low-threshold tags entity should be retained"
    );
}

/// Exact-match entity passes threshold boundary.
#[tokio::test]
async fn test_mine_session_boundary_at_threshold_keeps_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-bt".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Rust language basics", "subject")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("bt.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-bt", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "exact-match entity should pass threshold"
    );
}

/// All entities filtered → empty entity_names.
#[tokio::test]
async fn test_mine_session_all_entities_filtered() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-all".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event("X", crate::miner::MiningEventCategory::Error)];
    let entities = vec![vec![
        make_entity("January 2025", "time"),
        make_entity("March 2026", "time"),
        make_entity("December 2024", "time"),
    ]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("all.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-all", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        0,
        "all loosely-related entities should be filtered"
    );
}

// ── NgramEmbedder direct similarity verification ────────────────────

/// Verify that the integration test mock data (post-Step 1.1) produces
/// sufficient n-gram similarity to pass the action threshold (0.78).
///
/// Event: title="Wrong deployment", summary="Deployed to prod without testing"
/// Entity: name="wrong deployment", description="deployed without testing"
#[test]
fn test_mock_data_similarity_action_threshold() {
    let event_text = "Wrong deployment Deployed to prod without testing";
    let entity_text = "wrong deployment deployed without testing";
    let corpus: Vec<&str> = vec![event_text, entity_text];
    let emb = NgramEmbedder::new(&corpus);
    let event_emb = emb.embed(event_text);
    let entity_emb = emb.embed(entity_text);
    let sim = cosine_similarity(&event_emb, &entity_emb);
    assert!(
        sim >= 0.78,
        "action-type mock entity should pass 0.78 threshold, got {sim}"
    );
}

/// Verify dedup test mock data (post-Step 1.1) passes the subject
/// threshold (0.78).
///
/// Event: title="Test dedup event", summary="Same event testing"
/// Entity: name="same event testing", description="dedup"
#[test]
fn test_mock_data_similarity_subject_threshold() {
    let event_text = "Test dedup event Same event testing";
    let entity_text = "same event testing dedup";
    let corpus: Vec<&str> = vec![event_text, entity_text];
    let emb = NgramEmbedder::new(&corpus);
    let event_emb = emb.embed(event_text);
    let entity_emb = emb.embed(entity_text);
    let sim = cosine_similarity(&event_emb, &entity_emb);
    assert!(
        sim >= 0.78,
        "subject-type mock entity should pass 0.78 threshold, got {sim}"
    );
}

/// Completely unrelated entity text is filtered by any threshold.
#[test]
fn test_unrelated_entity_filtered_by_threshold() {
    let event_text = "Wrong deployment Deployed to prod without testing";
    let entity_text = "quantum physics superposition";
    let corpus: Vec<&str> = vec![event_text, entity_text];
    let emb = NgramEmbedder::new(&corpus);
    let event_emb = emb.embed(event_text);
    let entity_emb = emb.embed(entity_text);
    let sim = cosine_similarity(&event_emb, &entity_emb);
    assert!(
        sim < 0.70,
        "completely unrelated entity should have low similarity, got {sim}"
    );
}

/// Exact text match produces similarity ≈ 1.0, always passing any
/// threshold.
#[test]
fn test_exact_match_always_passes() {
    let text = "Wrong deployment Deployed to prod without testing";
    let corpus: Vec<&str> = vec![text];
    let emb = NgramEmbedder::new(&corpus);
    let a = emb.embed(text);
    let b = emb.embed(text);
    let sim = cosine_similarity(&a, &b);
    assert!(
        (sim - 1.0).abs() < 1e-10,
        "exact match should have similarity ~1.0, got {sim}"
    );
}

/// Partial overlap produces mid-range similarity (between 0.5 and 1.0).
#[test]
fn test_partial_overlap_mid_range() {
    let event_text = "Wrong deployment Deployed to prod without testing";
    let entity_text = "wrong deployment";
    let corpus: Vec<&str> = vec![event_text, entity_text];
    let emb = NgramEmbedder::new(&corpus);
    let event_emb = emb.embed(event_text);
    let entity_emb = emb.embed(entity_text);
    let sim = cosine_similarity(&event_emb, &entity_emb);
    assert!(
        sim > 0.5 && sim < 1.0,
        "partial overlap should be in (0.5, 1.0), got {sim}"
    );
}

// ── Per-entity-type threshold filtering tests ───────────────────────

/// Verify that the `action` type threshold (0.78) is correctly applied:
/// similar entity passes, dissimilar entity is filtered.
#[tokio::test]
async fn test_action_type_threshold_filtering() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-action".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Wrong deployment",
        crate::miner::MiningEventCategory::Error,
    )];
    // Two action entities: one similar (passes 0.78), one dissimilar.
    let entities = vec![vec![
        make_entity("wrong deployment", "action"),
        make_entity("quantum physics", "action"),
    ]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("action.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session(
            "sess-action",
            "Owner: hello\nAgent: response",
            "a1",
            &storage,
        )
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "only above-threshold action entity should be retained"
    );
    assert_eq!(result.entity_names[0][0], "wrong deployment");
}

/// Verify that the `tags` type threshold (0.70) is correctly applied:
/// a moderately similar entity passes the lower threshold.
#[tokio::test]
async fn test_tags_type_threshold_filtering() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-tags".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![make_entity("Rust language", "tags")]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("tags.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-tags", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "tags entity with moderate similarity should be retained"
    );
}

/// Verify that the `time` type threshold (0.90) is correctly applied:
/// loosely related entity is filtered by the high threshold.
#[tokio::test]
async fn test_time_type_threshold_filtering() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-time".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![MiningEntity {
        entity_type: "time".into(),
        name: "January 2025".into(),
        description: "a month in 2025".into(),
    }]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("time.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session("sess-time", "Owner: hello\nAgent: response", "a1", &storage)
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        0,
        "time entity with low similarity should be filtered"
    );
}

/// Boundary: near-exact match entity passes any threshold (>= comparison).
#[tokio::test]
async fn test_boundary_exact_threshold_keeps_entity() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-boundary".into());
    cp.mined = false;
    storage.add_checkpoint(cp);
    // Use exact-match text — entity name matches event title exactly.
    let events = vec![make_event(
        "Rust language basics",
        crate::miner::MiningEventCategory::Error,
    )];
    let entities = vec![vec![MiningEntity {
        entity_type: "subject".into(),
        name: "Rust language basics".into(),
        description: "Rust language basics".into(),
    }]];
    let config = MinerConfig {
        enabled: true,
        clean_rules: lenient_rules(),
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller {
        events_response: events,
        entities_response: entities,
        ..Default::default()
    });
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("boundary2.db");
    let miner = MemoryMiner::new(config, llm, &db_path, "memory.md");
    let result = miner
        .mine_session(
            "sess-boundary",
            "Owner: hello\nAgent: response",
            "a1",
            &storage,
        )
        .await
        .unwrap();
    assert_eq!(
        result.entity_names[0].len(),
        1,
        "near-exact match entity should be retained"
    );
}

/// All entity types have correct thresholds loaded from SQLite.
#[test]
fn test_all_entity_type_thresholds_correct() {
    let tmp = TempDir::new().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
    crate::miner::init_schema(&conn).unwrap();
    let thresholds = load_entity_type_thresholds(&conn).unwrap();
    assert_eq!(thresholds["time"], 0.90, "time threshold");
    assert_eq!(thresholds["location"], 0.75, "location threshold");
    assert_eq!(thresholds["person"], 0.80, "person threshold");
    assert_eq!(thresholds["organization"], 0.80, "organization threshold");
    assert_eq!(thresholds["subject"], 0.78, "subject threshold");
    assert_eq!(thresholds["product"], 0.80, "product threshold");
    assert_eq!(thresholds["metric"], 0.85, "metric threshold");
    assert_eq!(thresholds["action"], 0.78, "action threshold");
    assert_eq!(thresholds["work"], 0.80, "work threshold");
    assert_eq!(thresholds["group"], 0.78, "group threshold");
    assert_eq!(thresholds["tags"], 0.70, "tags threshold");
}
