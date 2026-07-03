//! Tests for the active-searcher runner module.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::active_searcher::{ActiveSearcherRunner, SessionMessageSnapshot};

    // ── Helpers ──────────────────────────────────────────────────────

    type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

    /// Minimal test runner: spawn with noop closures, return the runner.
    fn trigger_simple(
        session_id: &str,
        agent_id: &str,
        content: &str,
        message_role: &str,
        memory_db_path: Option<PathBuf>,
    ) -> ActiveSearcherRunner {
        let aid = agent_id.to_string();
        let get_agent_config = move |id: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > {
            let aid = aid.clone();
            Box::pin(async move {
                if id == aid {
                    Ok((Some("test-model".to_string()), None))
                } else {
                    Err("agent not found".to_string())
                }
            })
        };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let set_injection = |_sid: String,
                             _content: String,
                             _position: String,
                             _event_ids: Vec<i64>|
         -> BoxFuture<()> { Box::pin(async {}) };

        let run_searcher = |_db: String,
                            _aid: String,
                            _role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            Box::pin(async { None })
        };

        ActiveSearcherRunner::trigger(
            session_id,
            agent_id,
            content,
            message_role,
            &memory_db_path,
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        )
    }

    // ── Test: memory_db_path not set → no task spawned ──────────────

    #[tokio::test]
    async fn test_no_spawn_when_db_path_none() {
        let runner = trigger_simple("s1", "a1", "hello", "user", None);
        assert!(
            !runner.is_running(),
            "should not spawn task when memory_db_path is None"
        );
        let result: Result<(), _> = runner.join().await;
        assert!(result.is_ok());
    }

    // ── Test: memory_db_path set → task spawned ─────────────────────

    #[tokio::test]
    async fn test_spawn_when_db_path_set() {
        let db = PathBuf::from("/tmp/test.db");
        let runner = trigger_simple("s1", "a1", "hello", "user", Some(db));
        assert!(
            runner.is_running(),
            "should spawn task when memory_db_path is Some"
        );
        let result: Result<(), _> = runner.join().await;
        assert!(
            result.is_ok(),
            "background task should complete without error"
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

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let seen = Arc::clone(&seen_position);
        let set_injection = move |sid: String,
                                  content: String,
                                  position: String,
                                  _event_ids: Vec<i64>|
              -> BoxFuture<()> {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                assert_eq!(sid, session_id);
                assert_eq!(content, "user-search-result");
                *seen.lock().await = Some(position);
            })
        };

        // Simulate user message → run_searcher returns AfterCurrent position.
        let run_searcher = |_db: String,
                            _aid: String,
                            role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            assert_eq!(role, "user");
            Box::pin(async move {
                Some((
                    "user-search-result".to_string(),
                    "after_current".to_string(),
                    vec![],
                ))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            session_id,
            agent_id,
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        assert!(runner.is_running());
        let result: Result<(), _> = runner.join().await;
        result.unwrap();

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

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let seen = Arc::clone(&seen_position);
        let set_injection = move |sid: String,
                                  content: String,
                                  position: String,
                                  _event_ids: Vec<i64>|
              -> BoxFuture<()> {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                assert_eq!(sid, session_id);
                assert_eq!(content, "assistant-search-result");
                *seen.lock().await = Some(position);
            })
        };

        let run_searcher = |_db: String,
                            _aid: String,
                            role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            assert_eq!(role, "assistant");
            Box::pin(async move {
                Some((
                    "assistant-search-result".to_string(),
                    "before_next".to_string(),
                    vec![1, 2, 3],
                ))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            session_id,
            agent_id,
            "my response",
            "assistant",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        assert!(runner.is_running());
        let result: Result<(), _> = runner.join().await;
        result.unwrap();

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

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Err("agent not found".to_string()) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let set_injection = |_sid: String,
                             _content: String,
                             _position: String,
                             _event_ids: Vec<i64>|
         -> BoxFuture<()> {
            Box::pin(async {
                panic!("set_injection should not be called when agent config fails");
            })
        };

        let run_searcher = |_db: String,
                            _aid: String,
                            _role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            Box::pin(async {
                panic!("run_searcher should not be called when agent config fails");
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        // Task was spawned but should exit early due to config error.
        assert!(runner.is_running());
        let result: Result<(), _> = runner.join().await;
        result.unwrap();
        // If we reach here without panicking, graceful degradation works.
    }

    // ── Test: search returns None → no injection ────────────────────

    #[tokio::test]
    async fn test_search_returns_none_no_injection() {
        let db = PathBuf::from("/tmp/test.db");
        let injection_called: Arc<tokio::sync::Mutex<bool>> =
            Arc::new(tokio::sync::Mutex::new(false));

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let called = Arc::clone(&injection_called);
        let set_injection = move |_sid: String,
                                  _content: String,
                                  _position: String,
                                  _event_ids: Vec<i64>|
              -> BoxFuture<()> {
            let called = Arc::clone(&called);
            Box::pin(async move {
                *called.lock().await = true;
            })
        };

        let run_searcher = |_db: String,
                            _aid: String,
                            _role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            // Return None — no results found.
            Box::pin(async { None })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        let result: Result<(), _> = runner.join().await;
        result.unwrap();

        assert!(
            !*injection_called.lock().await,
            "set_injection should not be called when search returns None"
        );
    }

    // ── Test: cancel aborts the task ────────────────────────────────

    #[tokio::test]
    async fn test_cancel_aborts_task() {
        let db = PathBuf::from("/tmp/test.db");

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let set_injection = |_sid: String,
                             _content: String,
                             _position: String,
                             _event_ids: Vec<i64>|
         -> BoxFuture<()> { Box::pin(async {}) };

        // Slow searcher that takes a long time.
        let run_searcher = |_db: String,
                            _aid: String,
                            _role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Some(("r".to_string(), "after_current".to_string(), vec![]))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        assert!(runner.is_running());
        runner.cancel();

        // After cancel, join should return a JoinError (task was aborted).
        let result: Result<(), _> = runner.join().await;
        assert!(result.is_err(), "cancelled task should return JoinError");
    }

    // ── Test: run_searcher returns event IDs → they are forwarded ───

    #[tokio::test]
    async fn test_event_ids_forwarded_to_injection() {
        let db = PathBuf::from("/tmp/test.db");
        let seen_ids: Arc<tokio::sync::Mutex<Vec<i64>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let ids_ref = Arc::clone(&seen_ids);
        let set_injection = move |_sid: String,
                                  _content: String,
                                  _position: String,
                                  event_ids: Vec<i64>|
              -> BoxFuture<()> {
            let ids_ref = Arc::clone(&ids_ref);
            Box::pin(async move {
                *ids_ref.lock().await = event_ids;
            })
        };

        let run_searcher = |_db: String,
                            _aid: String,
                            _role: String,
                            _content: String,
                            _model: String,
                            _ctx: Vec<SessionMessageSnapshot>,
                            _ids: HashSet<i64>,
                            _cfg: serde_json::Value|
         -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            Box::pin(async {
                Some((
                    "summary".to_string(),
                    "after_current".to_string(),
                    vec![42, 99, 100],
                ))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        let result: Result<(), _> = runner.join().await;
        result.unwrap();

        let ids = seen_ids.lock().await;
        assert_eq!(*ids, vec![42, 99, 100]);
    }

    // ── Test: context messages are passed through ───────────────────

    #[tokio::test]
    async fn test_context_messages_passed_through() {
        let db = PathBuf::from("/tmp/test.db");
        let seen_ctx: Arc<tokio::sync::Mutex<Vec<SessionMessageSnapshot>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
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
        };

        let get_injected =
            |_sid: String| -> BoxFuture<HashSet<i64>> { Box::pin(async { HashSet::new() }) };

        let set_injection = |_sid: String,
                             _content: String,
                             _position: String,
                             _event_ids: Vec<i64>|
         -> BoxFuture<()> { Box::pin(async {}) };

        let ctx_ref = Arc::clone(&seen_ctx);
        let run_searcher = move |_db: String,
                                 _aid: String,
                                 _role: String,
                                 _content: String,
                                 _model: String,
                                 context: Vec<SessionMessageSnapshot>,
                                 _ids: HashSet<i64>,
                                 _cfg: serde_json::Value|
              -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            let ctx_ref = Arc::clone(&ctx_ref);
            Box::pin(async move {
                *ctx_ref.lock().await = context;
                Some(("r".to_string(), "after_current".to_string(), vec![]))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        let result: Result<(), _> = runner.join().await;
        result.unwrap();

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

        let get_agent_config = |_aid: String| -> BoxFuture<
            Result<(Option<String>, Option<serde_json::Value>), String>,
        > { Box::pin(async { Ok((Some("m".to_string()), None)) }) };

        let get_context = |_sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
            Box::pin(async { (Vec::new(), 20) })
        };

        let get_injected = |_sid: String| -> BoxFuture<HashSet<i64>> {
            let mut ids = HashSet::new();
            ids.insert(10);
            ids.insert(20);
            Box::pin(async move { ids })
        };

        let set_injection = |_sid: String,
                             _content: String,
                             _position: String,
                             _event_ids: Vec<i64>|
         -> BoxFuture<()> { Box::pin(async {}) };

        let ids_ref = Arc::clone(&seen_ids);
        let run_searcher = move |_db: String,
                                 _aid: String,
                                 _role: String,
                                 _content: String,
                                 _model: String,
                                 _ctx: Vec<SessionMessageSnapshot>,
                                 injected_ids: HashSet<i64>,
                                 _cfg: serde_json::Value|
              -> BoxFuture<Option<(String, String, Vec<i64>)>> {
            let ids_ref = Arc::clone(&ids_ref);
            Box::pin(async move {
                *ids_ref.lock().await = injected_ids;
                Some(("r".to_string(), "after_current".to_string(), vec![]))
            })
        };

        let runner = ActiveSearcherRunner::trigger(
            "s1",
            "a1",
            "hello",
            "user",
            &Some(db),
            get_agent_config,
            get_context,
            get_injected,
            set_injection,
            run_searcher,
        );

        let result: Result<(), _> = runner.join().await;
        result.unwrap();

        let ids = seen_ids.lock().await;
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert_eq!(ids.len(), 2);
    }

    // ── Test: SessionMessageSnapshot clone and debug ────────────────

    #[test]
    fn test_session_message_snapshot_traits() {
        let snap = SessionMessageSnapshot {
            role: "user".to_string(),
            content: "hello".to_string(),
        };
        let cloned = snap.clone();
        assert_eq!(cloned.role, "user");
        assert_eq!(cloned.content, "hello");
        // Debug should not panic.
        let _ = format!("{:?}", snap);
    }
}
