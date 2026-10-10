//! Active-searcher pipeline assembly for the Gateway.
//!
//! Composition-root side: wraps the memory crate's three-step pipeline
//! (`ActiveSearcherConfig::from_agent_config` → `ActiveSearcher::new` →
//! `ActiveSearcher::run`) into the [`SearcherRunner`] seam the Gateway
//! consumes. The Gateway itself never references the memory crate — it only
//! orchestrates when a search is triggered and where the result is injected.
//!
//! This module is also the config → memory-params mapping point for the
//! searcher path (memory owns its param types, config owns the resolved
//! config; both are wired here — pure field copies, defaults live in the
//! memory crate) plus the conversion between session message snapshots /
//! memory injection payloads and the tuple protocol of the seam.
//!
//! Both production paths (daemon startup / restart, and the CLI chat gateway
//! wired through the root crate) share this single implementation.

use std::collections::HashSet;
use std::sync::Arc;

use closeclaw_gateway::SearcherRunner;
use closeclaw_session::active_searcher::{SearcherInput, SessionMessageSnapshot};

/// LLM caller adapter for the active-searcher pipeline.
///
/// Wraps a [`closeclaw_common::LlmCaller`] so it can be used as a trait
/// object by the active-searcher pipeline in the memory crate.
pub struct ActiveSearcherLlmCaller {
    /// The common LLM caller used for prompt completion.
    pub caller: Arc<dyn closeclaw_common::LlmCaller>,
    /// Model identifier passed in the
    /// [`InternalRequest`](closeclaw_common::llm_types::InternalRequest).
    pub model: String,
}

#[async_trait::async_trait]
impl closeclaw_memory::active_searcher_llm::ActiveSearchLlm for ActiveSearcherLlmCaller {
    async fn complete(
        &self,
        prompt: &str,
    ) -> Result<String, closeclaw_memory::active_searcher::ActiveSearcherError> {
        use closeclaw_common::llm_types::{InternalMessage, InternalRequest};

        let request = InternalRequest {
            model: self.model.clone(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
                content_blocks: None,
                tool_call_id: None,
            }],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_common::ReasoningLevel::default(),
            turn_count: None,
        };

        match self.caller.call(request).await {
            Ok(response) => {
                let text = response
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        closeclaw_common::processor::ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                Ok(text)
            }
            Err(e) => Err(closeclaw_memory::active_searcher::ActiveSearcherError::Llm(
                e.to_string(),
            )),
        }
    }
}

/// Convert session message snapshots into the internal messages consumed
/// by the memory crate's active-searcher prompt builder.
pub(crate) fn snapshots_to_internal_messages(
    snapshots: &[SessionMessageSnapshot],
) -> Vec<closeclaw_common::llm_types::InternalMessage> {
    use closeclaw_common::llm_types::InternalMessage;
    use closeclaw_common::processor::ContentBlock;

    snapshots
        .iter()
        .map(|m| InternalMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            content_blocks: Some(vec![ContentBlock::Text(m.content.clone())]),
            tool_call_id: None,
        })
        .collect()
}

/// Map a memory-owned injection payload onto the tuple protocol used by
/// the session crate's `run_searcher` / `set_memory_injection` closures:
/// `(content, position_tag, injected_event_ids)`.
pub(crate) fn summary_to_slot_parts(
    summary: closeclaw_memory::active_searcher::InjectedMemorySummary,
) -> (String, String, HashSet<i64>) {
    use closeclaw_memory::active_searcher::MemorySummaryPosition;

    let position_tag = match summary.position {
        MemorySummaryPosition::BeforeNext => "before_next",
        MemorySummaryPosition::AfterCurrent => "after_current",
    };
    (
        summary.content,
        position_tag.to_string(),
        summary.injected_event_ids,
    )
}

/// Deserialize the memory config JSON into a strongly-typed struct.
pub(crate) fn deserialize_memory_config(
    memory_config: &serde_json::Value,
) -> Option<closeclaw_common::MemoryConfig> {
    serde_json::from_value(memory_config.clone()).ok()
}

/// Map the agent memory config onto the memory crate's search params.
///
/// Pure field copy — default-value fallbacks live in `closeclaw-memory`.
pub(crate) fn search_params_from_memory_config(
    mem_cfg: &closeclaw_common::MemoryConfig,
) -> closeclaw_memory::params::SearchParams {
    let search = &mem_cfg.search;
    closeclaw_memory::params::SearchParams {
        enabled: search.enabled,
        model: search.model.clone(),
        context_turns: search.context_turns,
        timeout_ms: search.timeout_ms,
        max_summary_chars: search.max_summary_chars,
        min_entity_hits: search.min_entity_hits,
        top_k_events: search.top_k_events,
    }
}

/// Map the agent forgetting config onto the memory crate's forgetting params.
pub(crate) fn forgetting_params_from_memory_config(
    mem_cfg: &closeclaw_common::MemoryConfig,
) -> closeclaw_memory::params::ForgettingParams {
    closeclaw_memory::params::ForgettingParams {
        injection_extension_days: mem_cfg.forgetting.injection_extension_days,
    }
}

/// Build the active-searcher config from model and memory config.
///
/// Returns `None` if `search.enabled` is `false` in the agent config.
pub(crate) fn build_searcher_config(
    model: &str,
    mem_cfg: &Option<closeclaw_common::MemoryConfig>,
) -> Option<closeclaw_memory::active_searcher::ActiveSearcherConfig> {
    use closeclaw_memory::active_searcher::ActiveSearcherConfig;
    let search = mem_cfg.as_ref().map(search_params_from_memory_config);
    let forgetting = mem_cfg.as_ref().map(forgetting_params_from_memory_config);
    ActiveSearcherConfig::from_agent_config(Some(model), search.as_ref(), forgetting.as_ref())
}

/// Execute the searcher pipeline and convert the result.
async fn run_searcher_pipeline(
    input: SearcherInput,
    caller: &ActiveSearcherLlmCaller,
) -> Option<(String, String, HashSet<i64>)> {
    use closeclaw_memory::active_searcher::ActiveSearcher;
    let llm_messages = snapshots_to_internal_messages(&input.context_messages);
    let mem_cfg = deserialize_memory_config(&input.memory_config);
    let config = build_searcher_config(&input.model, &mem_cfg)?;
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

    Some(summary_to_slot_parts(injection))
}

/// Build the [`SearcherRunner`] injected into the Gateway.
///
/// `caller` is the shared LLM caller of the process. The narrow
/// `ActiveSearchLlm::complete` surface carries only a prompt, so the model
/// of the resulting request is decided by the injected caller — here it is
/// hardcoded to an empty string and left for the unified fallback to resolve
/// from its configured chain entries. `SearcherInput::model` only feeds the
/// searcher config (see `build_searcher_config`), not the request's model
/// field.
pub fn build_searcher_runner(caller: Arc<dyn closeclaw_common::LlmCaller>) -> SearcherRunner {
    let llm = Arc::new(ActiveSearcherLlmCaller {
        caller,
        model: String::new(),
    });
    SearcherRunner::new(move |input: SearcherInput| {
        let llm = Arc::clone(&llm);
        Box::pin(async move { run_searcher_pipeline(input, &llm).await })
    })
}

#[cfg(test)]
#[path = "searcher_runner_tests.rs"]
mod tests;
