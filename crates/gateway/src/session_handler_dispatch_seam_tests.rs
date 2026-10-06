//! Composition-root seam tests for the active-searcher runner injection.
//!
//! The gateway never assembles the memory pipeline itself: the composition
//! root injects a [`SearcherRunner`]. These tests pin the two behavioral
//! branches of the seam — an injected runner receives the `SearcherInput`
//! and its result flows back to the session layer; without an injection the
//! searcher run yields no output (no injection is produced).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use closeclaw_session::active_searcher::SearcherInput;
use closeclaw_session::persistence::ReasoningLevel;

use super::SearcherTriggerDeps;
use crate::session_handler::SearcherRunner;
use crate::{GatewayConfig, SessionManager};

fn sample_input() -> SearcherInput {
    SearcherInput {
        db_path: "/nonexistent/memory.sqlite".to_string(),
        agent_id: "agent-a".to_string(),
        role: "user".to_string(),
        content: "recall the deployment runbook".to_string(),
        model: "test-model".to_string(),
        context_messages: Vec::new(),
        injected_ids: HashSet::new(),
        memory_config: serde_json::Value::Null,
    }
}

fn make_deps(runner: Option<SearcherRunner>) -> SearcherTriggerDeps {
    let config = GatewayConfig {
        name: "test-searcher-seam".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    SearcherTriggerDeps {
        session_manager: sm,
        searcher_runner: runner,
        memory_db_path: None,
        agent_model: None,
        memory_config: None,
    }
}

#[tokio::test]
async fn test_injected_searcher_runner_receives_input_and_returns_output() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_in_runner = Arc::clone(&seen);
    let runner = SearcherRunner::new(move |input: SearcherInput| {
        seen_in_runner
            .lock()
            .expect("lock")
            .push((input.agent_id, input.role, input.content));
        Box::pin(async move {
            Some((
                "recalled fact".to_string(),
                "before_next".to_string(),
                HashSet::from([7_i64]),
            ))
        })
    });

    let deps = make_deps(Some(runner));
    let output = deps.build_run_searcher()(sample_input()).await;

    assert_eq!(
        output,
        Some((
            "recalled fact".to_string(),
            "before_next".to_string(),
            HashSet::from([7_i64]),
        )),
        "the injected runner's result must flow back to the session layer"
    );
    assert_eq!(
        seen.lock().expect("lock").as_slice(),
        [(
            "agent-a".to_string(),
            "user".to_string(),
            "recall the deployment runbook".to_string()
        )],
        "the searcher input must reach the injected runner unchanged"
    );
}

#[tokio::test]
async fn test_absent_searcher_runner_yields_no_injection() {
    let deps = make_deps(None);
    let output = deps.build_run_searcher()(sample_input()).await;
    assert_eq!(
        output, None,
        "without a composition-root injection the searcher must produce no output"
    );
}
