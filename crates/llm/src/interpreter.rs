//! LLM Interpreter — protocol response normalisation and stream event mapping.
//!
//! Each [`ModelInterpreter`] implementation is bound to a specific LLM provider
//! and is responsible for converting the raw types produced by a
//! [`ChatProtocol`][crate::ChatProtocol] (`InternalResponse`, `StreamEvent`)
//! into the unified public types (`UnifiedResponse`, normalised `StreamEvent`).
//!
//! The [`InterpreterRegistry`] resolves a `(provider_id, model)` pair to the
//! appropriate interpreter by glob-pattern matching.

use crate::types::{
    ContentBlock, InternalResponse, RawContentBlock, StreamEvent, UnifiedResponse, UnifiedUsage,
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
            .map(Into::into)
            .collect();
        let usage = UnifiedUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            reasoning_tokens: response.usage.reasoning_tokens,
            cache_read_tokens: response.usage.cache_read_tokens,
            cache_write_tokens: response.usage.cache_write_tokens,
        };
        UnifiedResponse {
            content_blocks,
            usage,
            finish_reason: response.finish_reason,
            retry_attempts: 0,
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

// ─────────────────────────────────────────────────────────────────────────────
// Shared helper: OpenAI-compatible response interpretation
// ─────────────────────────────────────────────────────────────────────────────

/// Shared `interpret_response` logic for OpenAI-compatible providers.
///
/// Collects text, thinking, tool-use, and tool-result blocks from the raw
/// response, then applies the standard merging rule:
///
/// * If text is **empty** and thinking is non-empty → thinking is merged into a
///   single `ContentBlock::Text` (no `Thinking` block emitted).
/// * Otherwise → text and thinking are emitted as separate `Text` / `Thinking`
///   blocks.
///
/// When `preserve_signature` is `true`, the last non-`None` signature from
/// thinking blocks is forwarded into the `Thinking` block (used by DeepSeek and
/// MiniMax). When `false`, signature is always `None` (used by MiMo and GLM).
///
/// When `clear_cache_tokens` is `true`, `cache_read_tokens` and
/// `cache_write_tokens` are set to `None` in the output (used by GLM and
/// DeepSeek — providers that don't support cache tokens).
fn interpret_openai_compatible(
    response: InternalResponse,
    preserve_signature: bool,
    clear_cache_tokens: bool,
) -> UnifiedResponse {
    let mut text_parts: Vec<String> = vec![];
    let mut thinking_parts: Vec<String> = vec![];
    let mut last_signature: Option<String> = None;

    for block in response.content_blocks {
        match block {
            RawContentBlock::Text(s) => text_parts.push(s),
            RawContentBlock::Thinking {
                thinking: s,
                signature,
            } => {
                thinking_parts.push(s);
                if preserve_signature && signature.is_some() {
                    last_signature = signature;
                }
            }
            RawContentBlock::ToolUse { id, name, input } => {
                text_parts.push(format!("id:{id} name:{name} input:{input}"))
            }
            RawContentBlock::ToolResult {
                tool_call_id,
                content,
            } => text_parts.push(format!("tool_call_id:{tool_call_id} content:{content}")),
        }
    }

    let mut content_blocks: Vec<ContentBlock> = vec![];
    let text_empty = text_parts.iter().all(|s| s.is_empty());
    if text_empty && !thinking_parts.is_empty() {
        content_blocks.push(ContentBlock::Text(thinking_parts.join("")));
    } else {
        if !text_parts.iter().all(|s| s.is_empty()) {
            content_blocks.push(ContentBlock::Text(text_parts.join("")));
        }
        if !thinking_parts.is_empty() {
            content_blocks.push(ContentBlock::Thinking {
                thinking: thinking_parts.join(""),
                signature: if preserve_signature {
                    last_signature
                } else {
                    None
                },
            });
        }
    }

    UnifiedResponse {
        content_blocks,
        usage: UnifiedUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            reasoning_tokens: response.usage.reasoning_tokens,
            cache_read_tokens: if clear_cache_tokens {
                None
            } else {
                response.usage.cache_read_tokens
            },
            cache_write_tokens: if clear_cache_tokens {
                None
            } else {
                response.usage.cache_write_tokens
            },
        },
        finish_reason: response.finish_reason,
        retry_attempts: 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MinimaxInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Interpreter for MiniMax provider.
///
/// MiniMax uses `reasoning_content` in its raw response to carry chain-of-thought
/// content. When the text content is empty, the `reasoning_content` is mapped to a
/// [`ContentBlock::Text`] block. When both text and reasoning content are present,
/// they are emitted as separate [`ContentBlock::Text`] and [`ContentBlock::Thinking`]
/// blocks respectively.
#[derive(Clone, Copy, Debug, Default)]
pub struct MinimaxInterpreter;

impl ModelInterpreter for MinimaxInterpreter {
    fn name(&self) -> &str {
        "minimax"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        interpret_openai_compatible(response, true, false)
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
/// Maps `reasoning_content` to [`ContentBlock::Text`] when the regular `content`
/// is empty, following the same rule as other OpenAI-compatible interpreters:
/// if `content` is empty and `reasoning_content` is non-empty, the latter is
/// emitted as a single `ContentBlock::Text` block (no `Thinking` block).
#[derive(Clone, Copy, Debug, Default)]
pub struct GlmInterpreter;

impl ModelInterpreter for GlmInterpreter {
    fn name(&self) -> &str {
        "glm"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        interpret_openai_compatible(response, false, true)
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
/// support for reasoning models. When the text content is empty, the
/// `reasoning_content` is mapped to a [`ContentBlock::Text`] block.
/// When both text and reasoning content are present, they are emitted as
/// separate [`ContentBlock::Text`] and [`ContentBlock::Thinking`] blocks
/// respectively. This mirrors the merging rule used by [`MinimaxInterpreter`].
/// Signature fields are collected and forwarded to preserve traceability
/// (same pattern as [`MinimaxInterpreter`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct DeepSeekInterpreter;

impl ModelInterpreter for DeepSeekInterpreter {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        interpret_openai_compatible(response, true, true)
    }

    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MimoInterpreter
// ─────────────────────────────────────────────────────────────────────────────

/// Interpreter for MiMo provider.
///
/// MiMo uses an OpenAI-compatible wire format with `reasoning_content`
/// support. When the text content is empty, the `reasoning_content` is mapped
/// to a [`ContentBlock::Text`] block. When both text and reasoning content are
/// present, they are emitted as separate [`ContentBlock::Text`] and
/// [`ContentBlock::Thinking`] blocks respectively. Signature is always `None`
/// (MiMo characteristic — no signature field in the wire format).
///
/// MiMo supports prefix cache hits: `cache_read_tokens` and
/// `cache_write_tokens` from the upstream response are preserved as-is in
/// [`UnifiedUsage`]. No client-side `cache_control` annotation is required.
#[derive(Clone, Copy, Debug, Default)]
pub struct MimoInterpreter;

impl ModelInterpreter for MimoInterpreter {
    fn name(&self) -> &str {
        "mimo"
    }

    fn interpret_response(&self, response: InternalResponse) -> UnifiedResponse {
        interpret_openai_compatible(response, false, false)
    }

    fn interpret_stream_event(&self, event: StreamEvent) -> Option<StreamEvent> {
        Some(event)
    }
}
