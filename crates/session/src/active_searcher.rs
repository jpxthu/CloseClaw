//! Active-searcher runner for sessions.
//!
//! Provides [`ActiveSearcherRunner`] which encapsulates spawning a
//! background active-searcher task with lifecycle management
//! (`JoinHandle` tracking) and configurable timeout.
//!
//! The runner does not depend on any gateway or LLM crate types.
//! All external dependencies are injected via closures, keeping the
//! session crate at its layer in the dependency graph.

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::task::JoinHandle;

/// Pinned boxed future type used by dependency closures.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A lightweight snapshot of a session message, decoupled from
/// `closeclaw-llm::session::SessionMessage`.
///
/// Used by [`ActiveSearcherRunner`] to pass context messages across
/// crate boundaries without depending on the LLM crate.
#[derive(Debug, Clone)]
pub struct SessionMessageSnapshot {
    /// Role of the message sender (e.g. "user", "assistant").
    pub role: String,
    /// Plain-text content of the message.
    pub content: String,
}

/// Active-searcher runner with lifecycle management.
///
/// Wraps a `JoinHandle` so callers can track or cancel the background
/// searcher task. Created via [`ActiveSearcherRunner::trigger`].
pub struct ActiveSearcherRunner {
    /// Handle to the spawned background task, if any.
    handle: Option<JoinHandle<()>>,
}

impl ActiveSearcherRunner {
    /// Spawn a background active-searcher task for the given message.
    ///
    /// # Arguments
    ///
    /// * `session_id` – ID of the conversation session.
    /// * `agent_id` – ID of the agent owning this session.
    /// * `content` – The message content that triggered the search.
    /// * `message_role` – Role of the message (`"user"` or `"assistant"`).
    /// * `memory_db_path` – Path to the SQLite database for entity search.
    /// * `get_agent_config` – Async closure: given `agent_id`, returns
    ///   `(model: Option<String>, memory_config: Option<serde_json::Value>)`.
    /// * `get_context_messages` – Async closure: given `session_id`, returns
    ///   context messages as `(Vec<SessionMessageSnapshot>, context_turns)`.
    /// * `get_injected_event_ids` – Async closure: given `session_id`, returns
    ///   the set of already-injected event IDs.
    /// * `set_memory_injection` – Async closure: given `(session_id, content,
    ///   position, event_ids)`, writes the injection into the session slot.
    /// * `run_searcher` – Async closure: given all search parameters, executes
    ///   the full active-searcher pipeline and returns
    ///   `Option<(content, position, event_ids)>` on success.
    ///
    /// # Returns
    ///
    /// An [`ActiveSearcherRunner`] with a tracked `JoinHandle`. If `memory_db_path`
    /// is `None`, the handle is `None` and the task is not spawned.
    #[allow(clippy::too_many_arguments)]
    pub fn trigger<AC, CG, II, SI, RS>(
        session_id: &str,
        agent_id: &str,
        content: &str,
        message_role: &str,
        memory_db_path: &Option<PathBuf>,
        get_agent_config: AC,
        get_context_messages: CG,
        get_injected_event_ids: II,
        set_memory_injection: SI,
        run_searcher: RS,
    ) -> Self
    where
        AC: Fn(String) -> BoxFuture<Result<(Option<String>, Option<serde_json::Value>), String>>
            + Send
            + Sync
            + 'static,
        CG: Fn(String) -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> + Send + Sync + 'static,
        II: Fn(String) -> BoxFuture<HashSet<i64>> + Send + Sync + 'static,
        SI: Fn(String, String, String, Vec<i64>) -> BoxFuture<()> + Send + Sync + 'static,
        RS: Fn(
                String,
                String,
                String,
                String,
                String,
                Vec<SessionMessageSnapshot>,
                HashSet<i64>,
                serde_json::Value,
            ) -> BoxFuture<Option<(String, String, Vec<i64>)>>
            + Send
            + Sync
            + 'static,
    {
        let Some(ref db_path) = *memory_db_path else {
            return Self { handle: None };
        };

        let sid = session_id.to_string();
        let aid = agent_id.to_string();
        let content = content.to_string();
        let role = message_role.to_string();
        let db_path = db_path.clone();

        let handle = tokio::spawn(async move {
            // 1. Load agent config.
            let (agent_model, memory_config) = match get_agent_config(aid.clone()).await {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::warn!(
                        session_id = %sid,
                        error = %e,
                        "active-searcher: failed to load agent config"
                    );
                    return;
                }
            };

            // 2. Gather context messages and injected event IDs.
            let (context_messages, _context_turns) = get_context_messages(sid.clone()).await;
            let injected_ids = get_injected_event_ids(sid.clone()).await;

            // 3. Run the search pipeline (with timeout).
            let timeout_duration =
                std::time::Duration::from_millis(extract_timeout_ms(&memory_config));

            let result = tokio::time::timeout(
                timeout_duration,
                run_searcher(
                    db_path.to_string_lossy().to_string(),
                    aid,
                    role,
                    content,
                    agent_model.unwrap_or_default(),
                    context_messages,
                    injected_ids,
                    memory_config.unwrap_or(serde_json::Value::Null),
                ),
            )
            .await;

            match result {
                Ok(Some((inj_content, inj_position, inj_event_ids))) => {
                    set_memory_injection(sid, inj_content, inj_position, inj_event_ids).await;
                }
                Ok(None) => {
                    tracing::debug!(session_id = %sid, "active-searcher: no results");
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        session_id = %sid,
                        "active-searcher: timed out, abandoning this round"
                    );
                }
            }
        });

        Self {
            handle: Some(handle),
        }
    }

    /// Cancel the background task if it is still running.
    pub fn cancel(&self) {
        if let Some(ref h) = self.handle {
            h.abort();
        }
    }

    /// Returns `true` if the background task handle exists.
    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    /// Consume the runner and await the background task.
    ///
    /// Returns `Ok(())` if the task completed successfully, or an error
    /// if the task panicked or was cancelled.
    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        match self.handle {
            Some(h) => h.await,
            None => Ok(()),
        }
    }
}

/// Extract the timeout in milliseconds from the memory config JSON.
///
/// Looks for `active_searcher.timeout_ms` in the config object.
/// Falls back to 5000 ms (the default in `ActiveSearcherConfig`).
fn extract_timeout_ms(memory_config: &Option<serde_json::Value>) -> u64 {
    memory_config
        .as_ref()
        .and_then(|c| c.get("active_searcher"))
        .and_then(|c| c.get("timeout_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or(5000)
}
