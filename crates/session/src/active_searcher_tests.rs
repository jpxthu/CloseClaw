//! Tests for the active-searcher runner module.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::active_searcher::{
        spawn_active_searcher, SearcherDependencies, SearcherInput, SessionMessageSnapshot,
    };

    // ── Helpers ──────────────────────────────────────────────────────

    type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

    /// Build default noop dependencies for testing.
    fn noop_deps() -> SearcherDependencies {
        SearcherDependencies {
            get_agent_config: Box::new(|_id: String| -> BoxFuture<
                Result<(Option<String>, Option<serde_json::Value>), String>,
            > {
                Box::pin(async move { Ok((Some("test-model".to_string()), None)) })
            }),
            get_context_messages: Box::new(
                |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
                    Box::pin(async { (Vec::new(), 20) })
                },
            ),
            get_injected_event_ids: Box::new(
                |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) },
            ),
            set_memory_injection: Box::new(
                |_sid: String, _content: String, _position: String, _event_ids: HashSet<i64>| {
                    Box::pin(async {})
                },
            ),
            run_searcher: Box::new(|_input: SearcherInput| {
                Box::pin(async { None })
            }),
        }
    }

    // ── Test: memory_db_path not set → no task spawned ──────────────

    #[tokio::test]
    async fn test_no_spawn_when_db_path_none() {
        // With memory_db_path = None, spawn_active_searcher is a no-op.
        // We verify by confirming the injected flag was never set.
        let injection_called: Arc<tokio::sync::Mutex<bool>> =
            Arc::new(tokio::sync::Mutex::new(false));
        let called = Arc::clone(&injection_called);
        let mut deps = noop_deps();
        deps.run_searcher = Box::new(move |_input: SearcherInput| {
            let called = Arc::clone(&called);
            Box::pin(async move {
                *called.lock().await = true;
                None
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &None, deps);
        // Give a brief moment (shouldn't be needed since no spawn happens).
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !*injection_called.lock().await,
            "should not spawn task when memory_db_path is None"
        );
    }

    // ── Test: memory_db_path set → task spawns and runs ─────────────

    #[tokio::test]
    async fn test_spawn_when_db_path_set() {
        let db = PathBuf::from("/tmp/test.db");
        let task_ran: Arc<tokio::sync::Mutex<bool>> = Arc::new(tokio::sync::Mutex::new(false));
        let ran = Arc::clone(&task_ran);
        let mut deps = noop_deps();
        deps.run_searcher = Box::new(move |_input: SearcherInput| {
            let ran = Arc::clone(&ran);
            Box::pin(async move {
                *ran.lock().await = true;
                None
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        // Wait for the spawned task to complete.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            *task_ran.lock().await,
            "background task should have run when memory_db_path is Some"
        );
    }

    // ── Test: user message triggers AfterCurrent ────────────────────

    #[tokio::test]
    async fn test_user_message_after_current() {
        let db = PathBuf::from("/tmp/test.db");
        let session_id = "test-session-user";
        let agent_id = "test-agent-user";

        let seen_position: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let seen = Arc::clone(&seen_position);
        let mut deps = noop_deps();
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) });
        deps.set_memory_injection = Box::new(
            move |sid: String,
                  content: String,
                  position: String,
                  _event_ids: HashSet<i64>|
                  -> BoxFuture<()> {
                let seen = Arc::clone(&seen);
                Box::pin(async move {
                    assert_eq!(sid, session_id);
                    assert_eq!(content, "user-search-result");
                    *seen.lock().await = Some(position);
                })
            },
        );
        deps.run_searcher = Box::new(|input: SearcherInput| {
            assert_eq!(input.role, "user");
            Box::pin(async move {
                Some((
                    "user-search-result".to_string(),
                    "after_current".to_string(),
                    HashSet::new(),
                ))
            })
        });

        spawn_active_searcher(session_id, agent_id, "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let pos = seen_position.lock().await;
        assert_eq!(
            pos.as_deref(),
            Some("after_current"),
            "user message should write AfterCurrent"
        );
    }

    // ── Test: assistant message triggers BeforeNext ─────────────────

    #[tokio::test]
    async fn test_assistant_message_before_next() {
        let db = PathBuf::from("/tmp/test.db");
        let session_id = "test-session-assistant";
        let agent_id = "test-agent-assistant";

        let seen_position: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let seen = Arc::clone(&seen_position);
        let mut deps = noop_deps();
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) });
        deps.set_memory_injection = Box::new(
            move |sid: String,
                  content: String,
                  position: String,
                  _event_ids: HashSet<i64>|
                  -> BoxFuture<()> {
                let seen = Arc::clone(&seen);
                Box::pin(async move {
                    assert_eq!(sid, session_id);
                    assert_eq!(content, "assistant-search-result");
                    *seen.lock().await = Some(position);
                })
            },
        );
        deps.run_searcher = Box::new(|input: SearcherInput| {
            assert_eq!(input.role, "assistant");
            Box::pin(async move {
                let mut ids = HashSet::new();
                ids.insert(1);
                ids.insert(2);
                ids.insert(3);
                Some((
                    "assistant-search-result".to_string(),
                    "before_next".to_string(),
                    ids,
                ))
            })
        });

        spawn_active_searcher(
            session_id,
            agent_id,
            "my response",
            "assistant",
            &Some(db),
            deps,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        let pos = seen_position.lock().await;
        assert_eq!(
            pos.as_deref(),
            Some("before_next"),
            "assistant message should write BeforeNext"
        );
    }

    // ── Test: get_agent_config error → graceful degradation ─────────

    #[tokio::test]
    async fn test_graceful_degradation_on_agent_config_error() {
        let db = PathBuf::from("/tmp/test.db");
        let task_ran: Arc<tokio::sync::Mutex<bool>> = Arc::new(tokio::sync::Mutex::new(false));

        let ran = Arc::clone(&task_ran);
        let mut deps = noop_deps();
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Err("agent not found".to_string()) }) });
        deps.set_memory_injection = Box::new(
            |_sid: String, _content: String, _position: String, _event_ids: HashSet<i64>| {
                Box::pin(async {
                    panic!("set_injection should not be called when agent config fails");
                })
            },
        );
        deps.run_searcher = Box::new(move |_input: SearcherInput| {
            let ran = Arc::clone(&ran);
            Box::pin(async move {
                *ran.lock().await = true;
                panic!("run_searcher should not be called when agent config fails");
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;
        // If we reach here without panicking, graceful degradation works.
        assert!(
            !*task_ran.lock().await,
            "run_searcher should not be called when config loading fails"
        );
    }

    // ── Test: search returns None → no injection ────────────────────

    #[tokio::test]
    async fn test_search_returns_none_no_injection() {
        let db = PathBuf::from("/tmp/test.db");
        let injection_called: Arc<tokio::sync::Mutex<bool>> =
            Arc::new(tokio::sync::Mutex::new(false));

        let called = Arc::clone(&injection_called);
        let mut deps = noop_deps();
        deps.set_memory_injection = Box::new(
            move |_sid: String, _content: String, _position: String, _event_ids: HashSet<i64>| {
                let called = Arc::clone(&called);
                Box::pin(async move {
                    *called.lock().await = true;
                })
            },
        );
        deps.run_searcher = Box::new(|_input: SearcherInput| Box::pin(async { None }));

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !*injection_called.lock().await,
            "set_injection should not be called when search returns None"
        );
    }

    // ── Test: run_searcher returns event IDs → they are forwarded ───

    #[tokio::test]
    async fn test_event_ids_forwarded_to_injection() {
        let db = PathBuf::from("/tmp/test.db");
        let seen_ids: Arc<tokio::sync::Mutex<HashSet<i64>>> =
            Arc::new(tokio::sync::Mutex::new(HashSet::new()));

        let ids_ref = Arc::clone(&seen_ids);
        let mut deps = noop_deps();
        deps.set_memory_injection = Box::new(
            move |_sid: String,
                  _content: String,
                  _position: String,
                  event_ids: HashSet<i64>|
                  -> BoxFuture<()> {
                let ids_ref = Arc::clone(&ids_ref);
                Box::pin(async move {
                    *ids_ref.lock().await = event_ids;
                })
            },
        );
        deps.run_searcher = Box::new(|_input: SearcherInput| {
            Box::pin(async {
                let mut ids = HashSet::new();
                ids.insert(42);
                ids.insert(99);
                ids.insert(100);
                Some(("summary".to_string(), "after_current".to_string(), ids))
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let ids = seen_ids.lock().await;
        assert!(ids.contains(&42));
        assert!(ids.contains(&99));
        assert!(ids.contains(&100));
    }

    // ── Test: context messages are passed through ───────────────────

    #[tokio::test]
    async fn test_context_messages_passed_through() {
        let db = PathBuf::from("/tmp/test.db");
        let seen_ctx: Arc<tokio::sync::Mutex<Vec<SessionMessageSnapshot>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let ctx_ref = Arc::clone(&seen_ctx);
        let mut deps = noop_deps();
        deps.get_context_messages = Box::new(
            |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
                let msgs = vec![
                    SessionMessageSnapshot {
                        role: "user".to_string(),
                        content: "hello".to_string(),
                    },
                    SessionMessageSnapshot {
                        role: "assistant".to_string(),
                        content: "hi there".to_string(),
                    },
                ];
                Box::pin(async move { (msgs, 20) })
            },
        );
        deps.run_searcher = Box::new(move |input: SearcherInput| {
            let ctx_ref = Arc::clone(&ctx_ref);
            Box::pin(async move {
                *ctx_ref.lock().await = input.context_messages;
                Some(("r".to_string(), "after_current".to_string(), HashSet::new()))
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let ctx = seen_ctx.lock().await;
        assert_eq!(ctx.len(), 2);
        assert_eq!(ctx[0].role, "user");
        assert_eq!(ctx[0].content, "hello");
        assert_eq!(ctx[1].role, "assistant");
        assert_eq!(ctx[1].content, "hi there");
    }

    // ── Test: injected event IDs are passed through ─────────────────

    #[tokio::test]
    async fn test_injected_event_ids_passed_through() {
        let db = PathBuf::from("/tmp/test.db");
        let seen_ids: Arc<tokio::sync::Mutex<HashSet<i64>>> =
            Arc::new(tokio::sync::Mutex::new(HashSet::new()));

        let ids_ref = Arc::clone(&seen_ids);
        let mut deps = noop_deps();
        deps.get_injected_event_ids = Box::new(|_sid: String| -> BoxFuture<HashSet<i64>> {
            let mut ids = HashSet::new();
            ids.insert(10);
            ids.insert(20);
            Box::pin(async move { ids })
        });
        deps.run_searcher = Box::new(move |input: SearcherInput| {
            let ids_ref = Arc::clone(&ids_ref);
            Box::pin(async move {
                *ids_ref.lock().await = input.injected_ids;
                Some(("r".to_string(), "after_current".to_string(), HashSet::new()))
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let ids = seen_ids.lock().await;
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert_eq!(ids.len(), 2);
    }

    // ── Test: timeout triggers task abandonment ─────────────────────

    #[tokio::test]
    async fn test_timeout_triggers_abandonment() {
        let db = PathBuf::from("/tmp/test.db");
        let injection_called: Arc<tokio::sync::Mutex<bool>> =
            Arc::new(tokio::sync::Mutex::new(false));

        let called = Arc::clone(&injection_called);
        let mut deps = noop_deps();
        // Return memory_config with a very short timeout.
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > {
            let cfg = serde_json::json!({
                "search": { "timeout_ms": 1 }
            });
            Box::pin(async move { Ok((Some("m".to_string()), Some(cfg))) })
        });
        deps.set_memory_injection = Box::new(
            move |_sid: String, _content: String, _position: String, _event_ids: HashSet<i64>| {
                let called = Arc::clone(&called);
                Box::pin(async move {
                    *called.lock().await = true;
                })
            },
        );
        // Searcher sleeps longer than the timeout.
        deps.run_searcher = Box::new(|_input: SearcherInput| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Some(("r".to_string(), "after_current".to_string(), HashSet::new()))
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Injection should NOT have been called because the searcher timed out.
        assert!(
            !*injection_called.lock().await,
            "set_injection should not be called when searcher times out"
        );
    }
}
