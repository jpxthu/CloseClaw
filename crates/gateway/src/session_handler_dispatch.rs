//! LLM dispatch helpers for `SessionMessageHandler`.
//!
//! Extracted from `session_handler.rs` to keep the file under the
//! 500-line project limit. This module hosts the streaming-aware
//! dispatch path: [`SessionMessageHandler::handle_message_with_gateway`]
//! and [`SessionMessageHandler::dispatch_llm_call`], which route a
//! spawned LLM call to either the streaming pipeline (via
//! [`Gateway::send_outbound_streaming`]) or the non-streaming
//! fallback ([`SessionMessageHandler::call_llm`]).

use std::sync::Arc;

use super::session_handler::{
    flatten_content_blocks, FallbackLlmCaller, MessageMetadata, SessionMessageHandler,
};
use super::Gateway;
use crate::session_manager::SessionManager;
use crate::HandleResult;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::ChatSession;
use closeclaw_session::persistence::PendingMessage;

// ── Active-searcher trigger helpers ─────────────────────────────────

/// Shared dependencies for triggering the active-searcher background
/// task. Extracted so the trigger logic can be called from both the
/// pre-spawn path (user message) and inside the spawned task
/// (assistant message) without borrowing `&self` across `tokio::spawn`.
#[derive(Clone)]
struct SearcherTriggerDeps {
    session_manager: Arc<SessionManager>,
    fallback_llm_caller: Arc<FallbackLlmCaller>,
    memory_db_path: Option<std::path::PathBuf>,
    /// Pre-loaded agent model (avoids redundant config load in closures).
    agent_model: Option<String>,
    /// Pre-loaded memory config JSON (avoids redundant config load).
    memory_config: Option<serde_json::Value>,
}

type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

/// Return type for the `get_agent_config` closure.
type AgentConfigResult = Result<(Option<String>, Option<serde_json::Value>), String>;

type GetAgentConfig = Box<dyn Fn(String) -> BoxFuture<AgentConfigResult> + Send + Sync>;
type Snapshot = closeclaw_session::active_searcher::SessionMessageSnapshot;
type GetContextMessages = Box<dyn Fn(String) -> BoxFuture<(Vec<Snapshot>, usize)> + Send + Sync>;
type GetInjectedEventIds =
    Box<dyn Fn(String) -> BoxFuture<std::collections::HashSet<i64>> + Send + Sync>;
type SetMemoryInjection = Box<
    dyn Fn(String, String, String, std::collections::HashSet<i64>) -> BoxFuture<()> + Send + Sync,
>;

type RunSearcher = Box<
    dyn Fn(
            closeclaw_session::active_searcher::SearcherInput,
        ) -> BoxFuture<Option<(String, String, std::collections::HashSet<i64>)>>
        + Send
        + Sync,
>;

// ── Closure builders ───────────────────────────────────────────────

impl SearcherTriggerDeps {
    /// Build a closure that loads agent config from the session manager.
    ///
    /// Returns the pre-loaded config values (model, memory_config) that
    /// were passed into [`SearcherTriggerDeps`], avoiding a redundant
    /// config load inside the session crate.
    fn build_get_agent_config(&self) -> GetAgentConfig {
        let model = self.agent_model.clone();
        let mem_cfg = self.memory_config.clone();
        Box::new(move |_aid: String| -> BoxFuture<AgentConfigResult> {
            let model = model.clone();
            let mem_cfg = mem_cfg.clone();
            Box::pin(async move { Ok((model, mem_cfg)) })
        })
    }

    /// Build a closure that gathers context messages for a session.
    ///
    /// `context_turns` is passed from the caller (already extracted
    /// from the pre-loaded memory config) to avoid redundant config
    /// deserialization.
    fn build_get_context_messages(&self, context_turns: usize) -> GetContextMessages {
        let sm = Arc::clone(&self.session_manager);
        Box::new(move |sid: String| -> BoxFuture<(Vec<Snapshot>, usize)> {
            let sm = Arc::clone(&sm);
            let ctx_turns = context_turns;
            Box::pin(async move {
                if let Some(cs) = sm.get_conversation_session(&sid).await {
                    let cs_read = cs.read().await;
                    let msgs = ChatSession::messages(&*cs_read);
                    let start = if msgs.len() > ctx_turns {
                        msgs.len() - ctx_turns
                    } else {
                        0
                    };
                    let snapshots: Vec<Snapshot> = msgs[start..]
                        .iter()
                        .map(|m| Snapshot {
                            role: m.role.clone(),
                            content: flatten_content_blocks(&m.content_blocks),
                        })
                        .collect();
                    (snapshots, ctx_turns)
                } else {
                    (Vec::new(), ctx_turns)
                }
            })
        })
    }

    /// Build a closure that fetches already-injected event IDs for dedup.
    fn build_get_injected_event_ids(&self) -> GetInjectedEventIds {
        let sm = Arc::clone(&self.session_manager);
        Box::new(
            move |sid: String| -> BoxFuture<std::collections::HashSet<i64>> {
                let sm = Arc::clone(&sm);
                Box::pin(async move {
                    if let Some(cs) = sm.get_conversation_session(&sid).await {
                        let cs_read = cs.read().await;
                        let slot = cs_read
                            .memory_injection_arc()
                            .lock()
                            .expect("memory_injection lock poisoned");
                        slot.as_ref()
                            .map(|inj| inj.injected_event_ids.clone())
                            .unwrap_or_default()
                    } else {
                        std::collections::HashSet::new()
                    }
                })
            },
        )
    }
}

// ── Write/execute closure builders ─────────────────────────────────

impl SearcherTriggerDeps {
    /// Build a closure that writes a memory injection into the session slot.
    fn build_set_memory_injection(&self) -> SetMemoryInjection {
        let sm = Arc::clone(&self.session_manager);
        Box::new(
            move |sid: String,
                  content: String,
                  position: String,
                  event_ids: std::collections::HashSet<i64>|
                  -> BoxFuture<()> {
                let sm = Arc::clone(&sm);
                Box::pin(async move {
                    if let Some(cs) = sm.get_conversation_session(&sid).await {
                        let pos_mode = match position.as_str() {
                            "before_next" => closeclaw_llm::session::InjectionPosition::BeforeNext,
                            _ => closeclaw_llm::session::InjectionPosition::AfterCurrent,
                        };
                        let injection = closeclaw_llm::session::MemoryInjection {
                            content,
                            position_mode: pos_mode,
                            injected_event_ids: event_ids,
                        };
                        cs.read().await.set_memory_injection(injection);
                    }
                })
            },
        )
    }

    /// Build a closure that executes the active-searcher pipeline.
    fn build_run_searcher(&self) -> RunSearcher {
        let caller = Arc::clone(&self.fallback_llm_caller);
        Box::new(
            move |input: closeclaw_session::active_searcher::SearcherInput| -> BoxFuture<
                Option<(String, String, std::collections::HashSet<i64>)>,
            > {
                let caller = Arc::clone(&caller);
                Box::pin(async move {
                    run_searcher_pipeline(input, &caller).await
                })
            },
        )
    }
}

// ── build_run_searcher helpers ─────────────────────────────────────

/// Convert session message snapshots to LLM session messages.
fn convert_to_llm_messages(snapshots: &[Snapshot]) -> Vec<closeclaw_llm::session::SessionMessage> {
    snapshots
        .iter()
        .map(|m| closeclaw_llm::session::SessionMessage {
            role: m.role.clone(),
            content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(m.content.clone())],
            timestamp: chrono::Utc::now(),
        })
        .collect()
}

/// Deserialize the memory config JSON into a strongly-typed struct.
fn deserialize_memory_config(
    memory_config: &serde_json::Value,
) -> Option<closeclaw_config::agents::MemoryConfig> {
    serde_json::from_value(memory_config.clone()).ok()
}

/// Build the active-searcher config from model and memory config.
fn build_searcher_config(
    model: &str,
    mem_cfg: &Option<closeclaw_config::agents::MemoryConfig>,
) -> crate::memory::active_searcher::ActiveSearcherConfig {
    use crate::memory::active_searcher::ActiveSearcherConfig;
    ActiveSearcherConfig::from_agent_config(Some(model), mem_cfg.as_ref())
}

/// Execute the searcher pipeline and convert the result.
async fn run_searcher_pipeline(
    input: closeclaw_session::active_searcher::SearcherInput,
    caller: &FallbackLlmCaller,
) -> Option<(String, String, std::collections::HashSet<i64>)> {
    use crate::memory::active_searcher::ActiveSearcher;
    let llm_messages = convert_to_llm_messages(&input.context_messages);
    let mem_cfg = deserialize_memory_config(&input.memory_config);
    let config = build_searcher_config(&input.model, &mem_cfg);
    let searcher = ActiveSearcher::new(std::path::PathBuf::from(&input.db_path), config.clone());

    let injection = searcher
        .run(
            &input.agent_id,
            &input.role,
            &input.content,
            &llm_messages,
            &input.injected_ids,
            caller,
        )
        .await?;

    let pos_str = match injection.position_mode {
        closeclaw_llm::session::InjectionPosition::BeforeNext => "before_next".to_string(),
        closeclaw_llm::session::InjectionPosition::AfterCurrent => "after_current".to_string(),
    };
    Some((injection.content, pos_str, injection.injected_event_ids))
}

// ── Config loading helper ──────────────────────────────────────────

/// Load agent config and extract `context_turns` from memory config.
///
/// Returns `(model, memory_config, context_turns)`. The caller
/// pre-loads once and passes values to `trigger()`, eliminating
/// redundant config loads inside closure builders.
async fn load_agent_config_with_context_turns(
    session_manager: &SessionManager,
    agent_id: &str,
) -> (Option<String>, Option<serde_json::Value>, usize) {
    match session_manager.get_agent_config(agent_id).await {
        Some(cfg) => {
            let mem_json = cfg
                .memory
                .as_ref()
                .and_then(|m| serde_json::to_value(m).ok());
            let ctx_turns = closeclaw_session::active_searcher::extract_context_turns(&mem_json);
            (cfg.model.map(|m| m.primary), mem_json, ctx_turns)
        }
        None => (None, None, 10),
    }
}

// ── Trigger assembly ───────────────────────────────────────────────

impl SearcherTriggerDeps {
    /// Trigger an active-searcher background task for the given message.
    ///
    /// Pre-loaded `agent_model`, `memory_config`, and `context_turns`
    /// are used by closure builders to avoid redundant config loads
    /// inside the session crate.
    fn trigger(
        &self,
        session_id: &str,
        agent_id: &str,
        content: &str,
        message_role: &str,
        context_turns: usize,
    ) {
        use crate::memory::active_searcher_llm::should_trigger_role;

        if !should_trigger_role(message_role) {
            return;
        }

        let deps = closeclaw_session::active_searcher::SearcherDependencies {
            get_agent_config: self.build_get_agent_config(),
            get_context_messages: self.build_get_context_messages(context_turns),
            get_injected_event_ids: self.build_get_injected_event_ids(),
            set_memory_injection: self.build_set_memory_injection(),
            run_searcher: self.build_run_searcher(),
        };

        closeclaw_session::active_searcher::spawn_active_searcher(
            session_id,
            agent_id,
            content,
            message_role,
            &self.memory_db_path,
            deps,
        );
    }
}

impl SessionMessageHandler {
    /// Streaming-aware entry point used by [`Gateway::handle_inbound_message`].
    ///
    /// Same as [`handle_message_with_meta`](Self::handle_message_with_meta) but
    /// passes the [`Gateway`] reference and [`IMPlugin`] to the dispatch
    /// task so streaming LLM output can be routed through
    /// [`Gateway::send_outbound_streaming`].
    pub async fn handle_message_with_gateway(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
        gateway: &Arc<Gateway>,
        plugin: &Arc<dyn IMPlugin>,
    ) -> HandleResult {
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content, meta, Some(gateway), Some(plugin))
            .await
    }

    /// Dispatch an LLM call inside a `tokio::spawn` task.
    ///
    /// When both `gateway` and `plugin` are provided AND the session has
    /// `stream_enabled = true`, the streaming pipeline is used
    /// ([`Self::call_llm_streaming`]). Otherwise the non-streaming
    /// pipeline is used ([`Self::call_llm`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch_llm_call(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
        gateway: Option<&Arc<Gateway>>,
        plugin: Option<&Arc<dyn IMPlugin>>,
    ) -> HandleResult {
        self.set_busy(session_id, true).await;

        // ── Trigger active-searcher (best-effort, non-blocking) ────
        // Pre-load agent config once for both user and assistant
        // triggers, avoiding redundant config loads inside closures.
        let searcher_deps = SearcherTriggerDeps {
            session_manager: Arc::clone(&self.session_manager),
            fallback_llm_caller: Arc::clone(&self.fallback_llm_caller),
            memory_db_path: self.memory_db_path.clone(),
            agent_model: None,
            memory_config: None,
        };
        if let Some(agent_id) = self.session_manager.get_chat_id(session_id).await {
            let (model, mem_cfg, ctx_turns) =
                load_agent_config_with_context_turns(&self.session_manager, &agent_id).await;
            let deps = SearcherTriggerDeps {
                agent_model: model,
                memory_config: mem_cfg,
                ..searcher_deps.clone()
            };
            deps.trigger(session_id, &agent_id, &content, "user", ctx_turns);
        }

        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let llm_caller = Arc::clone(&self.llm_caller);
        let output_tx = Arc::clone(&self.output_tx);
        let channel = meta.channel.clone();
        // Clone the gateway/plugin into the spawn (optional). If
        // streaming is enabled for the session but no gateway/plugin
        // is provided we fall back to the non-streaming path.
        let gw_for_task = gateway.map(Arc::clone);
        let plugin_for_task = plugin.map(Arc::clone);
        // Clone the shutdown handle for busy-count tracking inside
        // the spawned task. The handle is optional (tests may not
        // set one).
        let shutdown_handle = self.shutdown_handle.clone();
        let searcher_deps = searcher_deps;

        tokio::spawn(async move {
            // Increment busy count at message dequeue.
            if let Some(ref h) = shutdown_handle {
                h.increment_busy();
            }

            // Check if streaming is enabled for this session
            let stream_enabled = if let Some(cs) = sm.get_conversation_session(&session_id).await {
                cs.read().await.stream_enabled()
            } else {
                false
            };

            let result = if stream_enabled {
                if let (Some(gw), Some(pl)) = (gw_for_task.as_ref(), plugin_for_task.as_ref()) {
                    Self::call_llm_streaming(
                        &llm_caller,
                        &content_for_task,
                        &meta,
                        &sm,
                        &session_id,
                        &channel,
                        gw,
                        pl,
                    )
                    .await
                } else {
                    tracing::warn!(
                        session_id = %session_id,
                        "streaming enabled but no gateway/plugin; \
                         falling back to non-streaming"
                    );
                    Self::call_llm(&llm_caller, &content_for_task, &meta, &sm, &session_id)
                        .await
                        .map(Into::into)
                }
            } else {
                Self::call_llm(&llm_caller, &content_for_task, &meta, &sm, &session_id)
                    .await
                    .map(Into::into)
            };

            // Extract assistant response text for active-searcher
            // trigger (BeforeNext mode for assistant messages).
            let assistant_text = match &result {
                Ok(sr) => sr
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        closeclaw_llm::types::ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
                Err(_) => String::new(),
            };

            Self::finish_llm(&sm, &session_id, result, &llm_caller, &output_tx).await;

            // ── Trigger active-searcher for assistant message ───
            // After the assistant response is stored in the session,
            // trigger active-searcher with role "assistant" so it
            // writes to the BeforeNext slot for the next user turn.
            if let Some(agent_id) = sm.get_chat_id(&session_id).await {
                let (model, mem_cfg, ctx_turns) =
                    load_agent_config_with_context_turns(&sm, &agent_id).await;
                let deps = SearcherTriggerDeps {
                    agent_model: model,
                    memory_config: mem_cfg,
                    ..searcher_deps.clone()
                };
                deps.trigger(
                    &session_id,
                    &agent_id,
                    &assistant_text,
                    "assistant",
                    ctx_turns,
                );
            }

            // Decrement busy count after response sent + pending
            // drained.
            if let Some(ref h) = shutdown_handle {
                h.decrement_busy();
            }
        });

        HandleResult::LlmStarted
    }

    pub(super) async fn set_busy(&self, session_id: &str, busy: bool) {
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.write().await;
            cs.set_llm_busy(busy);
            if busy {
                cs.set_llm_state(LlmState::Requesting);
            }
        }
    }

    pub(super) async fn enqueue_pending(&self, session_id: &str, content: String) {
        let msg = PendingMessage::new(
            format!("pending-{}", chrono::Utc::now().timestamp_millis()),
            content,
        );
        if let Err(e) = self
            .session_manager
            .push_pending_message(session_id, msg)
            .await
        {
            tracing::warn!(
                session_id,
                error = %e,
                "failed to enqueue pending message"
            );
        }
    }
}
