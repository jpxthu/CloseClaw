//! Lightweight LLM quality gate hooks for session turns.
//!
//! Hook 审查器在 turn 结束、硬规则通过后执行，对 turn 输出做
//! 质量门禁。每个 Hook 是固定 prompt 的轻量 LLM 调用，与主对话
//! 隔离，不进入 transcript。
//!
//! Design reference: `docs/design/session/run-health.md`.

use async_trait::async_trait;
pub use closeclaw_common::{HookConfig, HookType};
use futures::future::join_all;

use super::health_types::HookContext;

/// Returns the fixed prompt template for a hook type.
pub fn hook_prompt_template(hook_type: &HookType) -> &'static str {
    match hook_type {
        HookType::PlanCheck => {
            "You are reviewing an assistant turn for plan-only behavior. \
             The assistant made no tool calls. Does the text contain \
             promises, plans, or commitments without concrete action? \
             Respond with YES if the assistant only planned/promised \
             without executing, NO if the turn is acceptable."
        }
        HookType::LoopCheck => {
            "You are reviewing an assistant's recent tool call history \
             for repetitive loops. The assistant has been calling the \
             same tool with similar parameters across multiple turns \
             without meaningful progress. Respond with YES if there is \
             a repetitive loop pattern, NO if the tool calls show \
             genuine progress."
        }
        HookType::ProgressCheck => {
            "You are reviewing an assistant turn for verifiable progress. \
             Check whether the turn produced any file changes or \
             meaningful tool results. Respond with YES if no progress \
             was made, NO if the turn advanced the task."
        }
    }
}

/// Verdict from a single hook review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookVerdict {
    /// Whether the hook flagged the turn as unhealthy.
    pub flag: bool,
    /// Human-readable explanation for the verdict.
    pub reason: String,
    /// Which hook produced this verdict.
    pub hook_type: HookType,
}

/// Abstraction over the LLM provider used by hook reviews.
///
/// Production code injects a real LLM provider; tests inject a mock.
#[async_trait]
pub trait HookLlmProvider: Send + Sync {
    /// Run a single review call against the LLM.
    ///
    /// Returns `Ok(true)` if the hook flags the turn (unhealthy),
    /// `Ok(false)` if the turn is acceptable, or `Err` on failure.
    async fn review(&self, prompt: &str, context: &str) -> Result<bool, String>;
}

/// Orchestrates configured hook reviews for a session turn.
///
/// Holds the agent's hook configuration and an LLM provider for
/// executing review calls. Each hook runs sequentially; if any hook
/// flags the turn, the session is considered unhealthy.
pub struct HookReviewer {
    hooks: Vec<HookConfig>,
    llm: Box<dyn HookLlmProvider>,
}

impl HookReviewer {
    /// Create a new reviewer with the given hook configurations and
    /// LLM provider.
    pub fn new(hooks: Vec<HookConfig>, llm: Box<dyn HookLlmProvider>) -> Self {
        Self { hooks, llm }
    }

    /// Review a turn snapshot against all enabled hooks.
    ///
    /// Returns a verdict for each enabled hook. If no hooks are
    /// configured, returns an empty list. Hook verdicts are returned
    /// in configuration order.
    pub async fn review(&self, snapshot: &HookContext) -> Vec<HookVerdict> {
        let enabled_with_index: Vec<(usize, &HookConfig)> = self
            .hooks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled)
            .collect();

        if enabled_with_index.is_empty() {
            return Vec::new();
        }

        let futures: Vec<_> = enabled_with_index
            .iter()
            .map(|(_, config)| self.run_hook(&config.hook_type, snapshot))
            .collect();

        let results = join_all(futures).await;

        let mut indexed: Vec<(usize, HookVerdict)> = results
            .into_iter()
            .enumerate()
            .map(|(i, v)| (enabled_with_index[i].0, v))
            .collect();
        indexed.sort_by_key(|(idx, _)| *idx);

        indexed.into_iter().map(|(_, v)| v).collect()
    }

    /// Execute a single hook against the turn snapshot.
    async fn run_hook(&self, hook_type: &HookType, snapshot: &HookContext) -> HookVerdict {
        let context = format_turn_context(snapshot);
        let prompt = hook_prompt_template(hook_type);

        match self.llm.review(prompt, &context).await {
            Ok(flag) => HookVerdict {
                flag,
                reason: if flag {
                    format!("{hook_type:?} flagged the turn")
                } else {
                    format!("{hook_type:?} found no issues")
                },
                hook_type: hook_type.clone(),
            },
            Err(err) => HookVerdict {
                // LLM failure is treated as not-flagging (graceful degradation).
                flag: false,
                reason: format!("{hook_type:?} review failed: {err}"),
                hook_type: hook_type.clone(),
            },
        }
    }
}

/// Format turn snapshot into a context string for LLM review.
pub(crate) fn format_turn_context(snapshot: &HookContext) -> String {
    let mut parts = Vec::new();

    if !snapshot.text.is_empty() {
        parts.push(format!("Text: {}", snapshot.text));
    }

    if !snapshot.tool_calls.is_empty() {
        let calls: Vec<String> = snapshot
            .tool_calls
            .iter()
            .map(|tc| format!("{}({})", tc.name, tc.input))
            .collect();
        parts.push(format!("Tool calls: [{}]", calls.join(", ")));
    }

    if !snapshot.tool_results.is_empty() {
        let results: Vec<String> = snapshot
            .tool_results
            .iter()
            .enumerate()
            .map(|(i, r)| format!("Result {i}: {r}"))
            .collect();
        parts.push(format!("Tool results:\n{}", results.join("\n")));
    }

    if !snapshot.recent_tool_calls.is_empty() {
        let calls: Vec<String> = snapshot
            .recent_tool_calls
            .iter()
            .map(|tc| format!("{}({})", tc.name, tc.input))
            .collect();
        parts.push(format!("Recent tool calls: [{}]", calls.join(", ")));
    }

    parts.join("\n\n")
}
