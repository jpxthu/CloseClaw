//! Unit tests for concurrency count correctness (Step 1.1 / 1.3).
//!
//! Verifies that `count_active_children` returns correct values after
//! run-mode children complete and session-mode children are killed.

use super::spawn::SpawnMode;
use super::test_helpers::make_response;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::llm::session::ChatSession;
use crate::llm::types::ContentBlock;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::config::SubagentsConfig;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::BootstrapMode;

fn test_resolved_config(id: &str, workspace: Option<PathBuf>) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        permissions: None,
        source: ConfigSource::Merged,
    }
}

async fn register_parent_session(mgr: &SessionManager, parent_id: &str, workdir: PathBuf) {
    let cs = ConversationSession::new(parent_id.to_string(), "test-model".to_string(), workdir);
    let arc = Arc::new(RwLock::new(cs));
    let mut conv = mgr.conversation_sessions.write().await;
    conv.insert(parent_id.to_string(), arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
}

/// Run-mode child: after `try_push_announce` completes, the child is
/// removed from the `children` tracking table and `count_active_children`
/// returns 0.
#[tokio::test]
#[serial]
async fn test_run_mode_child_count_zero_after_announce() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("counter-child", None);

    register_parent_session(&mgr, "parent-count", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-count",
            1,
            "count task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .expect("create_child_session should succeed");

    // Verify child is tracked
    assert_eq!(mgr.count_active_children("parent-count").await, 1);

    // Simulate child completing: append an assistant message so
    // try_push_announce can extract result_text.
    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child should exist");
    cs.write()
        .await
        .append_response(make_response(vec![ContentBlock::Text("done".to_string())]));

    // Announce pushes event + removes from children table
    mgr.try_push_announce(&child_id).await;

    // Count should now be 0
    assert_eq!(
        mgr.count_active_children("parent-count").await,
        0,
        "count_active_children should return 0 after run-mode child announce"
    );
}

/// Session-mode child: after `kill_child`, `count_active_children` returns 0.
#[tokio::test]
#[serial]
async fn test_session_mode_child_count_zero_after_kill() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("kill-counter-child", None);

    register_parent_session(&mgr, "parent-kill-count", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-kill-count",
            1,
            "kill task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .expect("create_child_session should succeed");

    assert_eq!(mgr.count_active_children("parent-kill-count").await, 1);

    mgr.kill_child("parent-kill-count", &child_id)
        .await
        .expect("kill_child should succeed");

    assert_eq!(
        mgr.count_active_children("parent-kill-count").await,
        0,
        "count_active_children should return 0 after kill_child"
    );
}

/// Multiple run-mode children: count decrements correctly for each
/// completion, and the final count is 0 when all have announced.
#[tokio::test]
#[serial]
async fn test_multiple_run_mode_children_count_decrements_correctly() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    register_parent_session(&mgr, "parent-multi", tmp.path().to_path_buf()).await;

    const N: usize = 4;
    let mut child_ids: Vec<String> = Vec::with_capacity(N);

    for i in 0..N {
        let config = test_resolved_config(&format!("worker-{}", i), None);
        let child_id = mgr
            .create_child_session(
                &config,
                "parent-multi",
                1,
                &format!("task {}", i),
                true,
                None,
                SpawnMode::Run,
                false,
                None,
                None,
                None,
                3,
            )
            .await
            .expect("create_child_session should succeed");
        child_ids.push(child_id);
    }

    // All N children are tracked
    assert_eq!(mgr.count_active_children("parent-multi").await, N);

    // Complete children one by one, assert count after each
    for (i, child_id) in child_ids.iter().enumerate() {
        let cs = mgr
            .get_conversation_session(child_id)
            .await
            .expect("child should exist");
        cs.write()
            .await
            .append_response(make_response(vec![ContentBlock::Text(format!(
                "answer-{}",
                i
            ))]));

        mgr.try_push_announce(child_id).await;

        let expected_count = N - (i + 1);
        assert_eq!(
            mgr.count_active_children("parent-multi").await,
            expected_count,
            "after completing child {}, count should be {}",
            i,
            expected_count
        );
    }

    // Final count is 0
    assert_eq!(
        mgr.count_active_children("parent-multi").await,
        0,
        "count should be 0 after all run-mode children have announced"
    );
}
