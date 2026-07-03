//! Active-searcher runner for sessions.
//!
//! Provides [`ActiveSearcherRunner`] which encapsulates spawning a
//! background active-searcher task with lifecycle management
//! (`JoinHandle` tracking) and configurable timeout.
//!
//! The runner does not depend on any gateway or LLM crate types.
//! All external dependencies are injected via [`SearcherDependencies`],
//! keeping the session crate at its layer in the dependency graph.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
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

// ── Dependency types ────────────────────────────────────────────────────

/// Async closure: given `agent_id`, returns `(model, memory_config)`.
type GetAgentConfig = Box<
    dyn Fn(String) -> BoxFuture<Result<(Option<String>, Option<serde_json::Value>), String>>
        + Send
        + Sync,
>;

/// Async closure: given `session_id`, returns context messages.
type GetContextMessages =
    Box<dyn Fn(String) -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> + Send + Sync>;

/// Async closure: given `session_id`, returns already-injected event IDs.
type GetInjectedEventIds = Box<dyn Fn(String) -> BoxFuture<HashSet<i64>> + Send + Sync>;

/// Async closure: writes a memory injection into the session slot.
type SetMemoryInjection =
    Box<dyn Fn(String, String, String, Vec<i64>) -> BoxFuture<()> + Send + Sync>;

/// Async closure: executes the full active-searcher pipeline.
type RunSearcher = Box<
    dyn Fn(
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
        + Sync,
>;

/// All external dependencies needed by [`ActiveSearcherRunner::trigger`].
///
/// Bundles the five async closures that the runner depends on, keeping
/// the `trigger` signature within the 6-parameter limit.
pub struct SearcherDependencies {
    /// Load agent configuration (model + memory config).
    pub get_agent_config: GetAgentConfig,
    /// Fetch context messages for a session.
    pub get_context_messages: GetContextMessages,
    /// Fetch already-injected event IDs for a session.
    pub get_injected_event_ids: GetInjectedEventIds,
    /// Write a memory injection into the session slot.
    pub set_memory_injection: SetMemoryInjection,
    /// Execute the active-searcher pipeline.
    pub run_searcher: RunSearcher,
}

// ── Search context ──────────────────────────────────────────────────────

/// Context data bundled together for the search pipeline call.
///
/// Groups the scattered parameters that [`run_search_pipeline`] needs,
/// keeping its signature within the project's parameter limit.
struct SearchContext {
    /// Agent model name (may be `None`).
    agent_model: Option<String>,
    /// Context messages for the search.
    context_messages: Vec<SessionMessageSnapshot>,
    /// Already-injected event IDs (for dedup).
    injected_ids: HashSet<i64>,
    /// Memory config from the agent config.
    memory_config: Option<serde_json::Value>,
}

// ── ActiveSearcherRunner ────────────────────────────────────────────────

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
    /// * `deps` – All async dependency closures bundled together.
    ///
    /// # Returns
    ///
    /// An [`ActiveSearcherRunner`] with a tracked `JoinHandle`. If
    /// `memory_db_path` is `None`, the handle is `None` and the task
    /// is not spawned.
    pub fn trigger(
        session_id: &str,
        agent_id: &str,
        content: &str,
        message_role: &str,
        memory_db_path: &Option<PathBuf>,
        deps: SearcherDependencies,
    ) -> Self {
        let Some(ref db_path) = *memory_db_path else {
            return Self { handle: None };
        };

        let handle = spawn_search_task(
            session_id,
            agent_id,
            content,
            message_role,
            db_path.clone(),
            deps,
        );
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

// ── Helper functions ────────────────────────────────────────────────────

/// Spawn the background search task. Returns the `JoinHandle`.
fn spawn_search_task(
    session_id: &str,
    agent_id: &str,
    content: &str,
    message_role: &str,
    db_path: PathBuf,
    deps: SearcherDependencies,
) -> JoinHandle<()> {
    let sid = session_id.to_string();
    let aid = agent_id.to_string();
    let content = content.to_string();
    let role = message_role.to_string();

    tokio::spawn(async move {
        let (agent_model, memory_config) = match load_agent_config(&deps, &aid).await {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    agent_id = %aid,
                    error = %e,
                    "active-searcher: failed to load agent config"
                );
                return;
            }
        };

        let context_messages = fetch_context_messages(&deps, &sid).await;
        let injected_ids = fetch_injected_event_ids(&deps, &sid).await;

        let ctx = SearchContext {
            agent_model,
            context_messages,
            injected_ids,
            memory_config,
        };

        let result = run_search_pipeline(&deps, &db_path, &aid, &role, &content, &ctx).await;

        handle_search_result(&deps, &sid, result).await;
    })
}

/// Load agent config; returns `Ok((model, memory_config))`.
///
/// Returns `Err` if the agent config could not be loaded.
async fn load_agent_config(
    deps: &SearcherDependencies,
    agent_id: &str,
) -> Result<(Option<String>, Option<serde_json::Value>), String> {
    (deps.get_agent_config)(agent_id.to_string()).await
}

/// Fetch context messages for the session.
async fn fetch_context_messages(
    deps: &SearcherDependencies,
    session_id: &str,
) -> Vec<SessionMessageSnapshot> {
    let (msgs, _turns) = (deps.get_context_messages)(session_id.to_string()).await;
    msgs
}

/// Fetch already-injected event IDs for the session.
async fn fetch_injected_event_ids(deps: &SearcherDependencies, session_id: &str) -> HashSet<i64> {
    (deps.get_injected_event_ids)(session_id.to_string()).await
}

/// Run the searcher pipeline with timeout.
async fn run_search_pipeline(
    deps: &SearcherDependencies,
    db_path: &Path,
    agent_id: &str,
    role: &str,
    content: &str,
    ctx: &SearchContext,
) -> Option<(String, String, Vec<i64>)> {
    let timeout_duration = std::time::Duration::from_millis(extract_timeout_ms(&ctx.memory_config));

    let result = tokio::time::timeout(
        timeout_duration,
        (deps.run_searcher)(
            db_path.to_string_lossy().to_string(),
            agent_id.to_string(),
            role.to_string(),
            content.to_string(),
            ctx.agent_model.clone().unwrap_or_default(),
            ctx.context_messages.clone(),
            ctx.injected_ids.clone(),
            ctx.memory_config.clone().unwrap_or(serde_json::Value::Null),
        ),
    )
    .await;

    match result {
        Ok(inner) => inner,
        Err(_elapsed) => {
            tracing::warn!(
                session_id = %agent_id,
                "active-searcher: timed out, abandoning this round"
            );
            None
        }
    }
}

/// Handle the search result: write injection or log accordingly.
async fn handle_search_result(
    deps: &SearcherDependencies,
    session_id: &str,
    result: Option<(String, String, Vec<i64>)>,
) {
    match result {
        Some((inj_content, inj_position, inj_event_ids)) => {
            (deps.set_memory_injection)(
                session_id.to_string(),
                inj_content,
                inj_position,
                inj_event_ids,
            )
            .await;
        }
        None => {
            tracing::debug!(session_id = %session_id, "active-searcher: no results");
        }
    }
}

// ── Config extraction helpers ───────────────────────────────────────────

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

/// Extract `context_turns` from the memory config JSON.
///
/// Looks for `active_searcher.context_turns` in the config object.
/// Falls back to 10 turns.
pub fn extract_context_turns(memory_config: &Option<serde_json::Value>) -> usize {
    memory_config
        .as_ref()
        .and_then(|c| c.get("active_searcher"))
        .and_then(|c| c.get("context_turns"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(10)
}
