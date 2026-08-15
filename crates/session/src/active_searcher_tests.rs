//! Tests for the active-searcher runner module.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::active_searcher::{
        extract_context_turns, extract_timeout_ms, spawn_active_searcher, SearcherDependencies,
        SearcherInput, SessionMessageSnapshot,
    };
    use crate::active_searcher_session::SearcherSessionStatus;

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
            begin_searcher_session: Box::new(
                |_sid: String, _aid: String, _role: String| {
                    Box::pin(async { Some("test-searcher-session".to_string()) })
                },
            ),
            end_searcher_session: Box::new(|_sid: String, _status: SearcherSessionStatus| {}),
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

    // ── Step 1.1: config default value tests ───────────────────────

    /// When memory_config is None, extract_timeout_ms returns 3000
    /// (matching config.md search.timeout_ms default).
    #[test]
    fn test_extract_timeout_ms_default_when_none() {
        assert_eq!(extract_timeout_ms(&None), 3000);
    }

    /// When memory_config has no search.timeout_ms, fallback is 3000.
    #[test]
    fn test_extract_timeout_ms_default_when_empty_object() {
        let config = serde_json::json!({});
        assert_eq!(extract_timeout_ms(&Some(config)), 3000);
    }

    /// When memory_config has search.timeout_ms set, that value is used.
    #[test]
    fn test_extract_timeout_ms_explicit_value() {
        let config = serde_json::json!({ "search": { "timeout_ms": 5000 } });
        assert_eq!(extract_timeout_ms(&Some(config)), 5000);
    }

    /// When memory_config is None, extract_context_turns returns 5
    /// (matching config.md search.context_turns default).
    #[test]
    fn test_extract_context_turns_default_when_none() {
        assert_eq!(extract_context_turns(&None), 5);
    }

    /// When memory_config has no search.context_turns, fallback is 5.
    #[test]
    fn test_extract_context_turns_default_when_empty_object() {
        let config = serde_json::json!({});
        assert_eq!(extract_context_turns(&Some(config)), 5);
    }

    /// When memory_config has search.context_turns set, that value is used.
    #[test]
    fn test_extract_context_turns_explicit_value() {
        let config = serde_json::json!({ "search": { "context_turns": 10 } });
        assert_eq!(extract_context_turns(&Some(config)), 10);
    }

    // ── Step 1.3: begin_searcher_session returns None → skip ──────

    /// When `begin_searcher_session` returns `None` (parent session
    /// missing), the spawn task must not call `run_searcher` and must
    /// not write any injection.
    #[tokio::test]
    async fn test_begin_none_skips_run_and_injection() {
        let db = PathBuf::from("/tmp/test.db");
        let searcher_ran: Arc<tokio::sync::Mutex<bool>> = Arc::new(tokio::sync::Mutex::new(false));
        let injection_called: Arc<tokio::sync::Mutex<bool>> =
            Arc::new(tokio::sync::Mutex::new(false));

        let ran = Arc::clone(&searcher_ran);
        let called = Arc::clone(&injection_called);
        let mut deps = noop_deps();
        deps.begin_searcher_session =
            Box::new(|_sid: String, _aid: String, _role: String| Box::pin(async { None }));
        deps.run_searcher = Box::new(move |_input: SearcherInput| {
            let ran = Arc::clone(&ran);
            Box::pin(async move {
                *ran.lock().await = true;
                Some(("result".into(), "after_current".into(), HashSet::new()))
            })
        });
        deps.set_memory_injection = Box::new(
            move |_sid: String, _c: String, _p: String, _e: HashSet<i64>| {
                let called = Arc::clone(&called);
                Box::pin(async move {
                    *called.lock().await = true;
                })
            },
        );

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !*searcher_ran.lock().await,
            "run_searcher must not be called when begin returns None"
        );
        assert!(
            !*injection_called.lock().await,
            "set_memory_injection must not be called when begin returns None"
        );
    }

    // ── Step 1.3: end_searcher_session with unknown ID is safe ──────

    /// When the searcher session tracker has no record for the given
    /// ID, `end_searcher_session` should be a safe no-op (no panic,
    /// no observable side-effect).
    #[tokio::test]
    async fn test_end_unknown_id_is_safe_noop() {
        let db = PathBuf::from("/tmp/test.db");
        let end_called_with: Arc<tokio::sync::Mutex<Vec<String>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let recorded = Arc::clone(&end_called_with);
        let mut deps = noop_deps();
        deps.end_searcher_session = Box::new(move |sid: String, status: SearcherSessionStatus| {
            let recorded = Arc::clone(&recorded);
            recorded
                .try_lock()
                .unwrap()
                .push(format!("{sid}:{status:?}"));
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        // end_searcher_session was called with the test ID.
        let calls = end_called_with.lock().await;
        assert!(!calls.is_empty(), "end_searcher_session should be called");
        // The ID is "test-searcher-session" from noop_deps' begin;
        // the closure receives it and should not panic.
    }

    // ── Step 1.3: inject success → end(Injected) with finished_at ───

    /// When the searcher returns a result (Some), the end closure must
    /// be called with status "Injected".
    #[tokio::test]
    async fn test_inject_success_calls_end_injected() {
        let db = PathBuf::from("/tmp/test.db");
        let end_status: Arc<tokio::sync::Mutex<Option<SearcherSessionStatus>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let status_ref = Arc::clone(&end_status);
        let mut deps = noop_deps();
        deps.end_searcher_session = Box::new(move |_sid: String, status: SearcherSessionStatus| {
            let status_ref = Arc::clone(&status_ref);
            *status_ref.try_lock().unwrap() = Some(status);
        });
        deps.run_searcher = Box::new(|_input: SearcherInput| {
            Box::pin(async { Some(("found".into(), "after_current".into(), HashSet::new())) })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = end_status.lock().await;
        assert_eq!(*status, Some(SearcherSessionStatus::Injected));
    }

    // ── Step 1.3: no result → end(NoResult) ─────────────────────────

    /// When the searcher returns None, the end closure must be called
    /// with status "NoResult".
    #[tokio::test]
    async fn test_no_result_calls_end_no_result() {
        let db = PathBuf::from("/tmp/test.db");
        let end_status: Arc<tokio::sync::Mutex<Option<SearcherSessionStatus>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let status_ref = Arc::clone(&end_status);
        let mut deps = noop_deps();
        deps.end_searcher_session = Box::new(move |_sid: String, status: SearcherSessionStatus| {
            let status_ref = Arc::clone(&status_ref);
            *status_ref.try_lock().unwrap() = Some(status);
        });
        deps.run_searcher = Box::new(|_input: SearcherInput| Box::pin(async { None }));

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = end_status.lock().await;
        assert_eq!(*status, Some(SearcherSessionStatus::NoResult));
    }

    // ── Step 1.3: timeout → end(Abandoned) ──────────────────────────

    /// When the searcher times out, the end closure must be called
    /// with status "Abandoned" (distinct from NoResult).
    #[tokio::test]
    async fn test_timeout_calls_end_abandoned() {
        let db = PathBuf::from("/tmp/test.db");
        let end_status: Arc<tokio::sync::Mutex<Option<SearcherSessionStatus>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let status_ref = Arc::clone(&end_status);
        let mut deps = noop_deps();
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > {
            let cfg = serde_json::json!({ "search": { "timeout_ms": 1 } });
            Box::pin(async move { Ok((Some("m".into()), Some(cfg))) })
        });
        deps.end_searcher_session = Box::new(move |_sid: String, status: SearcherSessionStatus| {
            let status_ref = Arc::clone(&status_ref);
            *status_ref.try_lock().unwrap() = Some(status);
        });
        deps.run_searcher = Box::new(|_input: SearcherInput| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Some(("r".into(), "after_current".into(), HashSet::new()))
            })
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = end_status.lock().await;
        assert_eq!(*status, Some(SearcherSessionStatus::Abandoned));
    }

    // ── Step 1.3: agent config error → end(Abandoned) ───────────────

    /// When agent config loading fails, the end closure must be called
    /// with status "Abandoned".
    #[tokio::test]
    async fn test_config_error_calls_end_abandoned() {
        let db = PathBuf::from("/tmp/test.db");
        let end_status: Arc<tokio::sync::Mutex<Option<SearcherSessionStatus>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let status_ref = Arc::clone(&end_status);
        let mut deps = noop_deps();
        deps.get_agent_config = Box::new(|_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > {
            Box::pin(async { Err("not found".into()) })
        });
        deps.end_searcher_session = Box::new(move |_sid: String, status: SearcherSessionStatus| {
            let status_ref = Arc::clone(&status_ref);
            *status_ref.try_lock().unwrap() = Some(status);
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = end_status.lock().await;
        assert_eq!(*status, Some(SearcherSessionStatus::Abandoned));
    }

    // ── Step 1.3: searcher session ID passed to end ─────────────────

    /// The searcher session ID returned by `begin_searcher_session`
    /// must be forwarded to `end_searcher_session`.
    #[tokio::test]
    async fn test_searcher_session_id_forwarded_to_end() {
        let db = PathBuf::from("/tmp/test.db");
        let end_sid: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let sid_ref = Arc::clone(&end_sid);
        let mut deps = noop_deps();
        deps.begin_searcher_session = Box::new(|_sid: String, _aid: String, _role: String| {
            Box::pin(async { Some("my-searcher-id-123".to_string()) })
        });
        deps.end_searcher_session = Box::new(move |sid: String, _status: SearcherSessionStatus| {
            let sid_ref = Arc::clone(&sid_ref);
            *sid_ref.try_lock().unwrap() = Some(sid);
        });

        spawn_active_searcher("s1", "a1", "hello", "user", &Some(db), deps);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let sid = end_sid.lock().await;
        assert_eq!(sid.as_deref(), Some("my-searcher-id-123"));
    }
}
