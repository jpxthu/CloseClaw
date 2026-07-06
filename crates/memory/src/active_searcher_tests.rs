//! Unit tests for active-searcher module.
//!
//! Covers SQLite schema, search logic, event association, dedup,
//! summarise, memory_injection slot, role exclusion, and timeout.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection};

use crate::active_searcher::{
    ActiveSearcher, ActiveSearcherConfig, ActiveSearcherError, EventRecord,
};
use crate::active_searcher_llm::{parse_concepts, should_trigger_role, LlmCaller};
use closeclaw_llm::session::{InjectionPosition, MemoryInjection};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Create a temporary SQLite database with the standard schema.
fn create_test_db(dir: &Path) -> Connection {
    let db_path = dir.join("test.db");
    let conn = Connection::open(&db_path).unwrap();
    init_test_schema(&conn);
    conn
}

/// Initialize the test database with entity_types, entities, event_entities,
/// and an events table (not part of init_schema yet).
fn init_test_schema(conn: &Connection) {
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
fn insert_entity(
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
fn insert_event(conn: &Connection, content: &str, timestamp: i64, session_id: &str) -> i64 {
    conn.execute(
        "INSERT INTO events (content, timestamp, source_session_id)
         VALUES (?1, ?2, ?3)",
        params![content, timestamp, session_id],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Link an event to an entity.
fn link_event_entity(conn: &Connection, event_id: i64, entity_id: i64) {
    conn.execute(
        "INSERT INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
        params![event_id, entity_id],
    )
    .unwrap();
}

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
        params!["agent-1", "person", "Alice Alt", "alice"],
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
        id: 1, content: "A very long event description that should be truncated when exceeding the max summary chars limit".into(),
        timestamp: 1000, source_session_id: "s1".into(),
    }];
    assert!(searcher.summarize_events(&events).len() <= 50);
}

#[test]
fn test_memory_injection_basics() {
    let session = closeclaw_llm::session::ConversationSession::new(
        "test-session".into(),
        "model".into(),
        tempfile::tempdir().unwrap().keep(),
    );

    // slot empty initially
    assert!(session.take_memory_injection().is_none());

    // set and take
    let inj = MemoryInjection::new("summary".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(inj);
    let taken = session.take_memory_injection();
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().content, "summary");
    assert!(session.take_memory_injection().is_none());

    // position mode
    let after = MemoryInjection::new("a".into(), InjectionPosition::AfterCurrent);
    let before = MemoryInjection::new("b".into(), InjectionPosition::BeforeNext);
    assert_eq!(after.position_mode, InjectionPosition::AfterCurrent);
    assert_eq!(before.position_mode, InjectionPosition::BeforeNext);

    // event id dedup
    let mut inj = MemoryInjection::new("s".into(), InjectionPosition::AfterCurrent);
    assert!(!inj.is_event_injected(42));
    inj.add_injected_event_id(42);
    assert!(inj.is_event_injected(42));
    assert!(!inj.is_event_injected(99));
    inj.add_injected_event_id(42);
    assert_eq!(inj.injected_event_ids.len(), 1);

    // noop when empty slot
    let session2 = closeclaw_llm::session::ConversationSession::new(
        "test".into(),
        "model".into(),
        tempfile::tempdir().unwrap().keep(),
    );
    session2.add_injected_event_id(42);
    assert!(!session2.is_event_injected(42));
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

// ── Timeout test ─────────────────────────────────────────────────────────

struct SlowLlmCaller;

#[async_trait::async_trait]
impl LlmCaller for SlowLlmCaller {
    async fn complete(&self, _prompt: &str) -> Result<String, ActiveSearcherError> {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Ok("[]".into())
    }
}

#[tokio::test]
async fn test_run_timeout_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let config = ActiveSearcherConfig {
        timeout_ms: 50,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);
    let result = searcher
        .run(
            "agent-1",
            "user",
            "test message",
            &[],
            &HashSet::new(),
            &SlowLlmCaller,
        )
        .await;
    assert!(result.is_none(), "timeout should return None");
}

// ── LLM integration tests (mock) ────────────────────────────────────────

struct MockConceptLlm {
    concepts: Vec<String>,
}

#[async_trait::async_trait]
impl LlmCaller for MockConceptLlm {
    async fn complete(&self, _prompt: &str) -> Result<String, ActiveSearcherError> {
        let json = serde_json::to_string(&self.concepts).unwrap();
        Ok(json)
    }
}

#[tokio::test]
async fn test_run_full_pipeline_mock() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());

    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let ev1 = insert_event(&conn, "Alice met Bob", 1000, "sess-1");
    let ev2 = insert_event(&conn, "Alice went home", 2000, "sess-1");
    link_event_entity(&conn, ev1, eid);
    link_event_entity(&conn, ev2, eid);
    drop(conn);

    let config = ActiveSearcherConfig {
        timeout_ms: 5000,
        max_summary_chars: 500,
        min_entity_hits: 1,
        top_k_events: 10,
        context_turns: 3,
        model: "mock".into(),
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);
    let llm = MockConceptLlm {
        concepts: vec!["alice".into()],
    };

    let result = searcher
        .run(
            "agent-1",
            "user",
            "Tell me about Alice",
            &[],
            &HashSet::new(),
            &llm,
        )
        .await;

    assert!(result.is_some(), "pipeline should produce an injection");
    let injection = result.unwrap();
    assert!(!injection.content.is_empty(), "content should not be empty");
    assert_eq!(injection.position_mode, InjectionPosition::AfterCurrent);
    assert_eq!(injection.injected_event_ids.len(), 2);
}

#[tokio::test]
async fn test_run_excluded_role_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let llm = MockConceptLlm {
        concepts: vec!["test".into()],
    };
    let result = searcher
        .run(
            "agent-1",
            "memory-miner",
            "test",
            &[],
            &HashSet::new(),
            &llm,
        )
        .await;
    assert!(
        result.is_none(),
        "memory-miner should not trigger active searcher"
    );
}

#[tokio::test]
async fn test_run_empty_concepts_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let llm = MockConceptLlm { concepts: vec![] };
    let result = searcher
        .run("agent-1", "user", "test", &[], &HashSet::new(), &llm)
        .await;
    assert!(result.is_none(), "empty concepts should return None");
}

#[tokio::test]
async fn test_run_dedup_excludes_injected_events() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let eid = insert_entity(&conn, "agent-1", "person", "Alice", "alice");
    let ev1 = insert_event(&conn, "Alice met Bob", 1000, "sess-1");
    let ev2 = insert_event(&conn, "Alice went home", 2000, "sess-1");
    link_event_entity(&conn, ev1, eid);
    link_event_entity(&conn, ev2, eid);
    drop(conn);

    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), ActiveSearcherConfig::default());
    let llm = MockConceptLlm {
        concepts: vec!["alice".into()],
    };
    let mut injected = HashSet::new();
    injected.insert(ev1);

    let result = searcher
        .run("agent-1", "user", "test", &[], &injected, &llm)
        .await;

    if let Some(inj) = result {
        assert!(!inj.injected_event_ids.contains(&ev1));
        assert!(inj.injected_event_ids.contains(&ev2));
    }
}

// ── Concept parser tests ─────────────────────────────────────────────────

#[test]
fn test_parse_concepts() {
    let c = parse_concepts(r#"["Alice", "project X"]"#);
    assert_eq!(c, vec!["Alice", "project X"]);
    let c = parse_concepts(r#"Here are the concepts: ["Alice", "Bob"]"#);
    assert_eq!(c, vec!["Alice", "Bob"]);
    assert!(parse_concepts("[]").is_empty());
    assert!(parse_concepts("no json here").is_empty());
}

// ── Config defaults test ─────────────────────────────────────────────────

#[test]
fn test_active_searcher_config_defaults() {
    let config = ActiveSearcherConfig::default();
    assert_eq!(config.timeout_ms, 3000);
    assert_eq!(config.max_summary_chars, 500);
    assert_eq!(config.min_entity_hits, 1);
    assert_eq!(config.top_k_events, 3);
    assert_eq!(config.context_turns, 5);
    assert_eq!(config.model, "");
}

// ── from_agent_config tests ─────────────────────────────────────────────

use closeclaw_config::agents::{MemoryConfig, SearchConfig};

/// Full search config: all fields specified → all fields correct.
#[test]
fn test_from_agent_config_full_search_config() {
    let memory = MemoryConfig {
        search: SearchConfig {
            enabled: Some(true),
            model: Some("claude-opus".into()),
            timeout_ms: Some(9999),
            max_summary_chars: Some(8000),
            min_entity_hits: Some(5),
            top_k_events: Some(20),
            context_turns: Some(7),
        },
        ..MemoryConfig::default()
    };

    let config = ActiveSearcherConfig::from_agent_config(Some("gpt-4o"), Some(&memory));
    let config = config.expect("search should be enabled");

    assert_eq!(config.model, "claude-opus");
    assert_eq!(config.timeout_ms, 9999);
    assert_eq!(config.max_summary_chars, 8000);
    assert_eq!(config.min_entity_hits, 5);
    assert_eq!(config.top_k_events, 20);
    assert_eq!(config.context_turns, 7);
}

/// Partial search config: only model and timeout_ms → other fields use defaults.
#[test]
fn test_from_agent_config_partial_search_config() {
    let memory = MemoryConfig {
        search: SearchConfig {
            enabled: Some(true),
            model: Some("deepseek-r1".into()),
            timeout_ms: Some(12000),
            ..Default::default()
        },
        ..MemoryConfig::default()
    };

    let config = ActiveSearcherConfig::from_agent_config(None, Some(&memory));
    let config = config.expect("search should be enabled");

    assert_eq!(config.model, "deepseek-r1");
    assert_eq!(config.timeout_ms, 12000);
    // non-specified fields fall back to defaults
    assert_eq!(config.max_summary_chars, 500);
    assert_eq!(config.min_entity_hits, 1);
    assert_eq!(config.top_k_events, 3);
    assert_eq!(config.context_turns, 5);
}

/// No override (None) + agent_model → model uses agent global model.
#[test]
fn test_from_agent_config_no_override() {
    let config = ActiveSearcherConfig::from_agent_config(Some("gpt-4o-mini"), None).unwrap();
    assert_eq!(config.model, "gpt-4o-mini");
    // Without agent_model → model is empty string
    let config = ActiveSearcherConfig::from_agent_config(None, None).unwrap();
    assert_eq!(config.model, "");
}

/// Search config values flow through correctly.
#[test]
fn test_from_agent_config_search_config_values() {
    let memory = MemoryConfig {
        search: SearchConfig {
            enabled: Some(true),
            timeout_ms: Some(4000),
            max_summary_chars: Some(800),
            min_entity_hits: Some(2),
            top_k_events: Some(5),
            context_turns: Some(8),
            model: Some("search-model".into()),
            ..Default::default()
        },
        ..MemoryConfig::default()
    };
    let config = ActiveSearcherConfig::from_agent_config(Some("gpt-4o"), Some(&memory)).unwrap();
    assert_eq!(config.timeout_ms, 4000);
    assert_eq!(config.max_summary_chars, 800);
    assert_eq!(config.min_entity_hits, 2);
    assert_eq!(config.top_k_events, 5);
    assert_eq!(config.context_turns, 8);
    assert_eq!(config.model, "search-model");
}

/// search.model > agent_model.
#[test]
fn test_from_agent_config_model_priority() {
    let memory = MemoryConfig {
        search: SearchConfig {
            enabled: Some(true),
            model: Some("search-model".into()),
            ..Default::default()
        },
        ..MemoryConfig::default()
    };
    let config =
        ActiveSearcherConfig::from_agent_config(Some("agent-model"), Some(&memory)).unwrap();
    assert_eq!(config.model, "search-model");
}

/// Search disabled → from_agent_config returns None.
#[test]
fn test_from_agent_config_search_disabled() {
    let memory = MemoryConfig {
        search: SearchConfig {
            enabled: Some(false),
            ..Default::default()
        },
        ..MemoryConfig::default()
    };
    assert!(ActiveSearcherConfig::from_agent_config(Some("gpt-4o"), Some(&memory)).is_none());
}

// ── Concept extraction prompt dimension tests ─────────────────────────

use crate::active_searcher_llm::build_concept_extraction_prompt;
use chrono::Utc;
use closeclaw_llm::session::SessionMessage;
use closeclaw_llm::types::ContentBlock;

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
    let llm = SlowLlmCaller;

    let result = searcher
        .run(
            "agent-1",
            "user",
            "test message",
            &[],
            &HashSet::new(),
            &llm,
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
    let _ev_no_link = insert_event(&conn, "orphan event", 3000, "sess-1");
    let ev_linked = insert_event(&conn, "linked event", 2000, "sess-1");
    link_event_entity(&conn, ev_linked, eid);

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
    let ev = insert_event(&conn, "event", 1000, "sess-1");
    link_event_entity(&conn, ev, eid);

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

    let events = vec![EventRecord {
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
