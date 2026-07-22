use std::collections::HashSet;

use crate::active_searcher::{ActiveSearcher, ActiveSearcherConfig, EventRecord};

use super::{create_test_db, insert_entity, insert_event, link_event_entity};

// ── Event association tests ──────────────────────────────────────────────

#[test]
fn test_find_events_single_entity_multiple_events() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let ev1 = insert_event(&conn, "Alice did X", 1000, "sess-1");
    let ev2 = insert_event(&conn, "Alice did Y", 2000, "sess-1");
    link_event_entity(&conn, ev1, eid);
    link_event_entity(&conn, ev2, eid);

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let events = searcher.find_events(&[eid]).unwrap();
    assert_eq!(events.len(), 2);
    let ids: HashSet<i64> = events.iter().map(|e| e.id).collect();
    assert!(ids.contains(&ev1));
    assert!(ids.contains(&ev2));
}

#[test]
fn test_find_events_multiple_entities_dedup() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let e1 = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let e2 = insert_entity(&conn, "agent-1", "person", "Bob", "bob");
    let ev = insert_event(&conn, "Alice and Bob met", 1000, "sess-1");
    link_event_entity(&conn, ev, e1);
    link_event_entity(&conn, ev, e2);

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let events = searcher.find_events(&[e1, e2]).unwrap();
    assert_eq!(events.len(), 1, "same event should appear once");
    assert_eq!(events[0].id, ev);
}

#[test]
fn test_find_events_min_entity_hits_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let e1 = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let e2 = insert_entity(&conn, "agent-1", "person", "Bob", "bob");
    let ev1 = insert_event(&conn, "Alice alone", 1000, "sess-1");
    let ev2 = insert_event(&conn, "Alice and Bob", 2000, "sess-1");
    link_event_entity(&conn, ev1, e1);
    link_event_entity(&conn, ev2, e1);
    link_event_entity(&conn, ev2, e2);

    let config = ActiveSearcherConfig {
        min_entity_hits: 2,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    let events = searcher.find_events(&[e1, e2]).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, ev2);
}

#[test]
fn test_find_events_top_k_truncation() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    for i in 0..5 {
        let ev = insert_event(&conn, &format!("Event {i}"), i as i64 * 1000, "sess-1");
        link_event_entity(&conn, ev, eid);
    }

    let config = ActiveSearcherConfig {
        top_k_events: 3,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    let events = searcher.find_events(&[eid]).unwrap();
    assert_eq!(events.len(), 3, "should be limited to top_k_events");
}

#[test]
fn test_find_events_empty_entity_ids_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());

    let events = searcher.find_events(&[]).unwrap();
    assert!(events.is_empty());
}

// ── Dedup tests ──────────────────────────────────────────────────────────

#[test]
fn test_dedup_events() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let events = vec![
        EventRecord {
            id: 1,
            content: "ev1".into(),
            timestamp: 1000,
            source_session_id: "s1".into(),
        },
        EventRecord {
            id: 2,
            content: "ev2".into(),
            timestamp: 2000,
            source_session_id: "s1".into(),
        },
        EventRecord {
            id: 3,
            content: "ev3".into(),
            timestamp: 3000,
            source_session_id: "s1".into(),
        },
    ];
    // Empty injected set → all events pass
    let result = searcher.dedup_events(events.clone(), &HashSet::new());
    assert_eq!(result.len(), 3);
    // Inject id=2 → excluded from result
    let mut injected = HashSet::new();
    injected.insert(2);
    let result = searcher.dedup_events(events, &injected);
    assert_eq!(result.len(), 2);
    let ids: Vec<i64> = result.iter().map(|e| e.id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
}

// ── Summarize tests ──────────────────────────────────────────────────────

#[test]
fn test_summarize_events() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    // empty events → empty summary
    assert!(searcher.summarize_events(&[]).is_empty());
    // short text preserved
    let events = vec![EventRecord {
        id: 1,
        content: "Short event".into(),
        timestamp: 1000,
        source_session_id: "s1".into(),
    }];
    let summary = searcher.summarize_events(&events);
    assert!(summary.contains("Short event"));
    assert!(summary.contains("1000"));
    // long text truncated
    let config = ActiveSearcherConfig {
        max_summary_chars: 50,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);
    let events = vec![EventRecord {
        id: 1,
        content: "A very long event description that should be truncated when exceeding the max summary chars limit".into(),
        timestamp: 1000,
        source_session_id: "s1".into(),
    }];
    assert!(searcher.summarize_events(&events).len() <= 50);
}

// ── Memory injection basics ──────────────────────────────────────────

#[test]
fn test_memory_injection_basics() {
    let session = closeclaw_session::llm_session::ConversationSession::new(
        "test-session".into(),
        "model".into(),
        tempfile::tempdir().unwrap().keep(),
    );

    // slot empty initially
    assert!(session.take_memory_injection().is_none());

    // set and take
    let inj = closeclaw_session::llm_session::MemoryInjection::new(
        "summary".into(),
        closeclaw_session::llm_session::InjectionPosition::AfterCurrent,
    );
    session.set_memory_injection(inj);
    let taken = session.take_memory_injection();
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().content, "summary");
    assert!(session.take_memory_injection().is_none());

    // position mode
    let after = closeclaw_session::llm_session::MemoryInjection::new(
        "a".into(),
        closeclaw_session::llm_session::InjectionPosition::AfterCurrent,
    );
    let before = closeclaw_session::llm_session::MemoryInjection::new(
        "b".into(),
        closeclaw_session::llm_session::InjectionPosition::BeforeNext,
    );
    assert_eq!(
        after.position_mode,
        closeclaw_session::llm_session::InjectionPosition::AfterCurrent
    );
    assert_eq!(
        before.position_mode,
        closeclaw_session::llm_session::InjectionPosition::BeforeNext
    );

    // event id dedup
    let mut inj = closeclaw_session::llm_session::MemoryInjection::new(
        "s".into(),
        closeclaw_session::llm_session::InjectionPosition::AfterCurrent,
    );
    assert!(!inj.is_event_injected(42));
    inj.add_injected_event_id(42);
    assert!(inj.is_event_injected(42));
    assert!(!inj.is_event_injected(99));
    inj.add_injected_event_id(42);
    assert_eq!(inj.injected_event_ids.len(), 1);

    // noop when empty slot
    let session2 = closeclaw_session::llm_session::ConversationSession::new(
        "test".into(),
        "model".into(),
        tempfile::tempdir().unwrap().keep(),
    );
    session2.add_injected_event_id(42);
    assert!(!session2.is_event_injected(42));
}
