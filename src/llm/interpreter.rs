//! LLM Interpreter — protocol response normalisation and stream event mapping.
//!
//! Each [`ModelInterpreter`] implementation is bound to a specific LLM provider
//! and is responsible for converting the raw types produced by a
//! [`ChatProtocol`][crate::llm::ChatProtocol] (`InternalResponse`, `StreamEvent`)
//! into the unified public types (`UnifiedResponse`, normalised `StreamEvent`).
//!
//! The [`InterpreterRegistry`] resolves a `(provider_id, model)` pair to the
//! appropriate interpreter by glob-pattern matching.

use crate::llm::types::{
    ContentBlock, ContentBlockType, InternalRequest, InternalResponse, RawContentBlock, RawUsage,
    StreamEvent, UnifiedResponse, UnifiedUsage,
};

use glob::Pattern;

// ─────────────────────────────────────────────────────────────────────────────
// ModelInterpreter trait
// ─────────────────────────────────────────────────────────────────────────────

/// Trait for provider-specific response and stream-event normalisation.
///
/// Implementors translate protocol-internal raw types
/// ([`RawContentBlock`], [`RawUsage`], [`StreamEvent`]) into the unified public
/// types ([`ContentBlock`], [`UnifiedUsage`], normalised `StreamEvent`).
///
/// All implementations must be `Send + Sync` to allow shared access in the
/// client call pipeline.
pub trait ModelInterpreter: Send + Sync {
    /// Returns the identifier of the interpreter, typically matching the provider name.
    fn name(&self) -> &str;

    /// Converts an [`InternalResponse`] (produced by a `ChatProtocol`) into a
    /// [`UnifiedResponse`] (the public API response type).
    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse;

    /// Maps a raw [`StreamEvent`] through provider-specific logic.
    ///
    /// Returns `Some(normalised_event)` when the event should be forwarded,
    /// or `None` when the event should be suppressed.
    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent>;

    /// Injects additional fields into an [`InternalRequest`] before it is sent
    /// to the provider (e.g. provider-specific `extra_body` values).
    ///
    /// The default implementation is a no-op.
    fn inject_extra_body(&self, _request: &mut InternalRequest) {}
}

// ─────────────────────────────────────────────────────────────────────────────
// DefaultInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Default interpreter that performs an identity transformation.
///
/// Maps `RawContentBlock` → `ContentBlock` and `RawUsage` → `UnifiedUsage`
/// directly, without any provider-specific transformation.
/// Used as the fallback when no specialised interpreter matches.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultInterpreter;

impl ModelInterpreter for DefaultInterpreter {
    fn name(&self) -> &str {
        "default"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        let content_blocks: Vec<ContentBlock> = response
            .content_blocks
            .into_iter()
            .map(raw_content_block_to_content_block)
            .collect();
        let usage = UnifiedUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            reasoning_tokens: None,
        };
        UnifiedResponse {
            content_blocks,
            usage,
            finish_reason: response.finish_reason,
        }
    }

    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// InterpreterRegistry
// ─────────────────────────────────────────────────────────────────────────────

/// Entry in the interpreter registry: a glob pattern → interpreter binding.
struct RegistryEntry {
    pattern: Pattern,
    interpreter: Box<dyn ModelInterpreter>,
}

/// Registry for resolving provider/model pairs to [`ModelInterpreter`] instances.
///
/// Resolution is performed by matching the `provider_id/model` string against each
/// binding's glob pattern in registration order; the first match wins.
/// If no pattern matches, [`DefaultInterpreter`] is returned.
pub struct InterpreterRegistry {
    entries: Vec<RegistryEntry>,
    default_interpreter: DefaultInterpreter,
}

impl std::fmt::Debug for InterpreterRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterpreterRegistry")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl InterpreterRegistry {
    /// Creates a new registry from a list of (glob, interpreter) bindings.
    ///
    /// # Glob pattern format
    /// - `provider/*` — matches all models of a specific provider (e.g. `minimax/*`).
    /// - `provider/model` — matches a specific model exactly.
    /// - `*/*` — matches everything (catch-all).
    ///
    /// # Ordering
    /// Bindings are evaluated in the order supplied; place specific patterns before general ones.
    pub fn new(bindings: Vec<(Box<dyn ModelInterpreter>, &str)>) -> Self {
        let entries = bindings
            .into_iter()
            .map(|(interpreter, glob)| RegistryEntry {
                pattern: Pattern::new(glob)
                    .expect("invalid glob pattern in InterpreterRegistry::new"),
                interpreter,
            })
            .collect();
        Self {
            entries,
            default_interpreter: DefaultInterpreter,
        }
    }

    /// Resolves the appropriate interpreter for the given `(provider_id, model)`.
    ///
    /// Returns the first matching interpreter in registration order,
    /// or [`DefaultInterpreter`] if no pattern matches.
    pub fn resolve(&self, provider_id: &str, model: &str) -> &dyn ModelInterpreter {
        let target = format!("{}/{}", provider_id, model);
        for entry in &self.entries {
            if entry.pattern.matches(&target) {
                return &*entry.interpreter;
            }
        }
        &self.default_interpreter
    }
}

impl Default for InterpreterRegistry {
    fn default() -> Self {
        Self::new(vec![])
    }
}

fn raw_content_block_to_content_block(raw: RawContentBlock) -> ContentBlock {
    match raw {
        RawContentBlock::Text(s) => ContentBlock::Text(s),
        RawContentBlock::Thinking(s) => ContentBlock::Thinking(s),
        RawContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse { id, name, input },
        RawContentBlock::ToolResult {
            tool_call_id,
            content,
        } => ContentBlock::ToolResult {
            tool_call_id,
            content,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MinimaxInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Interpreter for MiniMax provider.
///
/// MiniMax uses `reasoning_content` in its raw response to carry chain-of-thought
/// content. When the text content is empty, the `reasoning_content` is mapped to a
/// [`ContentBlock::Thinking`] block.
#[derive(Clone, Copy, Debug, Default)]
pub struct MinimaxInterpreter;

impl ModelInterpreter for MinimaxInterpreter {
    fn name(&self) -> &str {
        "minimax"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        let mut text_parts: Vec<String> = vec![];
        let mut thinking_parts: Vec<String> = vec![];
        for block in response.content_blocks {
            match block {
                RawContentBlock::Text(s) => text_parts.push(s),
                RawContentBlock::Thinking(s) => thinking_parts.push(s),
                RawContentBlock::ToolUse { id, name, input } => {
                    text_parts.push(format!("id:{id} name:{name} input:{input}"))
                }
                RawContentBlock::ToolResult {
                    tool_call_id,
                    content,
                } => text_parts.push(format!("tool_call_id:{tool_call_id} content:{content}")),
            }
        }
        let content_blocks: Vec<ContentBlock> = if text_parts.iter().all(|s| s.is_empty()) {
            if !thinking_parts.is_empty() {
                vec![ContentBlock::Thinking(thinking_parts.join(""))]
            } else {
                vec![]
            }
        } else {
            vec![ContentBlock::Text(text_parts.join(""))]
        };
        UnifiedResponse {
            content_blocks,
            usage: UnifiedUsage {
                prompt_tokens: response.usage.prompt_tokens,
                completion_tokens: response.usage.completion_tokens,
                total_tokens: response.usage.total_tokens,
                reasoning_tokens: None,
            },
            finish_reason: response.finish_reason,
        }
    }

    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GlmInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Interpreter for GLM (Zhipu AI) provider.
///
/// Similar to MinimaxInterpreter but adds a threshold check: `reasoning_content`
/// is only promoted to a [`ContentBlock::Thinking`] block when its length
/// exceeds 10 bytes. Shorter reasoning content is treated as normal text.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlmInterpreter;

impl ModelInterpreter for GlmInterpreter {
    fn name(&self) -> &str {
        "glm"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        let mut text_parts: Vec<String> = vec![];
        let mut thinking_parts: Vec<String> = vec![];
        for block in response.content_blocks {
            match block {
                RawContentBlock::Text(s) => text_parts.push(s),
                RawContentBlock::Thinking(s) => thinking_parts.push(s),
                RawContentBlock::ToolUse { id, name, input } => {
                    text_parts.push(format!("id:{id} name:{name} input:{input}"))
                }
                RawContentBlock::ToolResult {
                    tool_call_id,
                    content,
                } => text_parts.push(format!("tool_call_id:{tool_call_id} content:{content}")),
            }
        }
        let content_blocks: Vec<ContentBlock> = if text_parts.iter().all(|s| s.is_empty()) {
            let reasoning = thinking_parts.join("");
            if reasoning.len() > 10 {
                vec![ContentBlock::Thinking(reasoning)]
            } else {
                vec![ContentBlock::Text(reasoning)]
            }
        } else {
            vec![ContentBlock::Text(text_parts.join(""))]
        };
        UnifiedResponse {
            content_blocks,
            usage: UnifiedUsage {
                prompt_tokens: response.usage.prompt_tokens,
                completion_tokens: response.usage.completion_tokens,
                total_tokens: response.usage.total_tokens,
                reasoning_tokens: None,
            },
            finish_reason: response.finish_reason,
        }
    }

    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DeepSeekInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Interpreter for DeepSeek provider.
///
/// DeepSeek uses an OpenAI-compatible wire format with `reasoning_content`
/// support for reasoning models. Since the protocol layer already normalises
/// the response into `RawContentBlock` variants, this interpreter applies the
/// same identity transformation as [`DefaultInterpreter`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DeepSeekInterpreter;

impl ModelInterpreter for DeepSeekInterpreter {
    fn name(&self) -> &str {
        "deepseek"
    }
    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        DefaultInterpreter.interpret_response(response)
    }
    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}
