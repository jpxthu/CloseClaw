use std::collections::HashSet;

use crate::active_searcher::ActiveSearcherError;
use crate::active_searcher::{ActiveSearcher, ActiveSearcherConfig};
use crate::active_searcher_llm::{build_concept_extraction_prompt, LlmCaller};
use chrono::Utc;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::SessionMessage;

use super::{create_test_db, insert_entity};

// ── Concept extraction prompt dimension tests ─────────────────────────

/// Verify the concept extraction prompt covers all three dimensions
/// defined in the design doc: action types, entities/objects,
/// and scenario characteristics; also includes message content.
#[test]
fn test_concept_extraction_prompt_coverage() {
    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![ContentBlock::Text("context info".into())],
        timestamp: Utc::now(),
    }];
    let prompt = build_concept_extraction_prompt(&messages, "current msg");
    let lower = prompt.to_lowercase();
    assert!(lower.contains("action"));
    assert!(lower.contains("entit") || lower.contains("object"));
    assert!(
        lower.contains("scenario") || lower.contains("context") || lower.contains("characteristic")
    );
    assert!(prompt.contains("context info"));
    assert!(prompt.contains("current msg"));
    assert!(prompt.contains("assistant: context info"));
}

// ── Boundary value tests ───────────────────────────────────────────

/// timeout_ms=0 causes immediate timeout, pipeline returns None.
#[tokio::test]
async fn test_run_timeout_zero_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let config = ActiveSearcherConfig {
        timeout_ms: 0,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    struct ImmediateTimeoutLlm;
    #[async_trait::async_trait]
    impl LlmCaller for ImmediateTimeoutLlm {
        async fn complete(&self, _prompt: &str) -> Result<String, ActiveSearcherError> {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok("[]".into())
        }
    }

    let result = searcher
        .run(
            "agent-1",
            "user",
            "test message",
            &[],
            &HashSet::new(),
            &ImmediateTimeoutLlm,
        )
        .await;

    assert!(
        result.is_none(),
        "timeout_ms=0 should return None immediately"
    );
}

/// min_entity_hits=0 means all events with >= 0 hits pass (all events).
#[test]
fn test_find_events_min_entity_hits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    // Create event with NO entity links
    let _ev_no_link = super::insert_event(&conn, "orphan event", 3000, "sess-1");
    let ev_linked = super::insert_event(&conn, "linked event", 2000, "sess-1");
    super::link_event_entity(&conn, ev_linked, eid);

    let config = ActiveSearcherConfig {
        min_entity_hits: 0,
        top_k_events: 10,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    let events = searcher.find_events(&[eid]).unwrap();
    // min_entity_hits=0: HAVING hit_count >= 0 passes all grouped events.
    // But only events with at least one entity link appear in the JOIN result.
    assert!(events.iter().any(|e| e.id == ev_linked));
}

/// top_k_events=0 returns no events.
#[test]
fn test_find_events_top_k_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let ev = super::insert_event(&conn, "event", 1000, "sess-1");
    super::link_event_entity(&conn, ev, eid);

    let config = ActiveSearcherConfig {
        top_k_events: 0,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    let events = searcher.find_events(&[eid]).unwrap();
    assert!(events.is_empty(), "top_k_events=0 should return no events");
}

/// max_summary_chars=0 produces empty summary.
#[test]
fn test_summarize_max_summary_chars_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let config = ActiveSearcherConfig {
        max_summary_chars: 0,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    let events = vec![crate::active_searcher::EventRecord {
        id: 1,
        content: "event".into(),
        timestamp: 1000,
        source_session_id: "s1".into(),
    }];
    let summary = searcher.summarize_events(&events);
    assert!(
        summary.is_empty(),
        "max_summary_chars=0 should produce empty summary"
    );
}

// ── Similarity threshold filtering tests ───────────────────────────

/// Exact match (normalized_name == concept) always passes threshold.
#[test]
fn test_search_entities_exact_match_passes_high_threshold() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "time", "rust language", "rust language");
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["rust language".into()])
        .unwrap();
    assert_eq!(results.len(), 1, "exact match should pass 0.90 threshold");
}

/// LIKE fuzzy match filtered when entity differs significantly.
#[test]
fn test_search_entities_fuzzy_match_filtered_by_high_threshold() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(
        &conn,
        "agent-1",
        "time",
        "rust language basics",
        "rust language basics",
    );
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["python".into()])
        .unwrap();
    assert!(results.is_empty(), "no LIKE match expected");
}

/// Low-threshold type retains loosely related fuzzy match.
#[test]
fn test_search_entities_low_threshold_keeps_fuzzy_match() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "tags", "programming", "programming");
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["program".into()])
        .unwrap();
    assert_eq!(results.len(), 1, "low threshold should retain fuzzy match");
}

/// Multiple concepts: threshold filtering applies per-entity.
#[test]
fn test_search_entities_mixed_threshold_results() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    insert_entity(&conn, "agent-1", "time", "rust lang", "rust lang");
    insert_entity(
        &conn,
        "agent-1",
        "tags",
        "rust programming",
        "rust programming",
    );
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let results = searcher
        .search_entities("agent-1", &["rust".into()])
        .unwrap();
    assert!(results.len() <= 2, "should have at most 2 results");
    assert!(
        results.iter().any(|e| e.entity_type == "tags"),
        "tags entity should pass low threshold"
    );
}
