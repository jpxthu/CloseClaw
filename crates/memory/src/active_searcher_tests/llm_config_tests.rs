use std::collections::HashSet;

use crate::active_searcher::{ActiveSearcher, ActiveSearcherConfig, ActiveSearcherError};
use crate::active_searcher_llm::LlmCaller;
use closeclaw_session::llm_session::InjectionPosition;

use super::{create_test_db, insert_entity, insert_event, link_event_entity};

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
    let c = crate::active_searcher_llm::parse_concepts(r#"["Alice", "project X"]"#);
    assert_eq!(c, vec!["Alice", "project X"]);
    let c =
        crate::active_searcher_llm::parse_concepts(r#"Here are the concepts: ["Alice", "Bob"]"#);
    assert_eq!(c, vec!["Alice", "Bob"]);
    assert!(crate::active_searcher_llm::parse_concepts("[]").is_empty());
    assert!(crate::active_searcher_llm::parse_concepts("no json here").is_empty());
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
