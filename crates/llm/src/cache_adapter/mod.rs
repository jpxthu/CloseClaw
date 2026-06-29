//! Cache adapter layer for LLM provider-specific prompt caching strategies.
//!
//! Different LLM providers expose different caching mechanisms. This module
//! provides a [`CacheAdapter`] trait and implementations for each provider's
//! caching strategy. The adapter runs *before* the Plugin Pipeline, acting as
//! an independent pre-processing step on the request hot path.

use std::sync::Arc;

use crate::types::{InternalRequest, SystemBlock};

/// Adapter trait for provider-specific prompt caching strategies.
///
/// Implementations transform an [`InternalRequest`] in place, injecting
/// provider-specific cache parameters (e.g., Anthropic `cache_control`,
/// Kimi `prompt_cache_key`).
pub trait CacheAdapter: Send + Sync {
    /// Returns the adapter name (for logging / diagnostics).
    fn name(&self) -> &str;

    /// Apply provider-specific caching transformations to the request.
    fn apply(&self, request: &mut InternalRequest);
}

/// No-op adapter — passes requests through unchanged.
///
/// Used for providers that do not support explicit prompt caching
/// (e.g., OpenAI, DeepSeek).
pub struct NoopCacheAdapter;

impl CacheAdapter for NoopCacheAdapter {
    fn name(&self) -> &str {
        "noop"
    }

    fn apply(&self, _request: &mut InternalRequest) {
        // intentionally empty
    }
}

/// Anthropic cache adapter — splits system prompt into cacheable blocks.
///
/// Generates structured [`SystemBlock`] entries from `system_static` and
/// `system_dynamic`, marking static blocks with `cache: true` so that the
/// Anthropic protocol layer can emit `cache_control: {"type": "ephemeral"}`.
///
/// Tool definitions (ToolsSection) are part of `system_static` and
/// therefore automatically covered by prefix caching through this adapter.
/// When tools are embedded in the system prompt text, no separate
/// `cache_control` annotation on the `tools` API parameter is needed.
pub struct AnthropicCacheAdapter;

impl CacheAdapter for AnthropicCacheAdapter {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn apply(&self, request: &mut InternalRequest) {
        let mut blocks = Vec::new();

        if let Some(ref static_text) = request.system_static {
            if !static_text.is_empty() {
                // Split static content by section (double newline) to get
                // finer-grained cache breakpoints.
                for section in static_text.split("\n\n") {
                    let trimmed = section.trim();
                    if !trimmed.is_empty() {
                        blocks.push(SystemBlock {
                            text: trimmed.to_owned(),
                            cache: true,
                        });
                    }
                }
            }
        }

        if let Some(ref dynamic_text) = request.system_dynamic {
            if !dynamic_text.is_empty() {
                blocks.push(SystemBlock {
                    text: dynamic_text.clone(),
                    cache: false,
                });
            }
        }

        if !blocks.is_empty() {
            request.system_blocks = Some(blocks);
        }

        // Mark all tool schemas as cacheable when tools are passed via the
        // API `tools` parameter (as opposed to being embedded in the system
        // prompt text, which is already covered by static block caching).
        if let Some(ref mut tools) = request.tools {
            for tool in tools.iter_mut() {
                tool.cache = true;
            }
        }
    }
}

/// Kimi cache adapter — injects `prompt_cache_key` into `extra_body`.
///
/// Uses the request's `session_id` as the cache key so that the Kimi
/// service-side automatic prefix cache can associate requests from the
/// same session.
pub struct KimiCacheAdapter;

impl CacheAdapter for KimiCacheAdapter {
    fn name(&self) -> &str {
        "kimi"
    }

    fn apply(&self, request: &mut InternalRequest) {
        if let Some(ref session_id) = request.session_id {
            request.extra_body.insert(
                "prompt_cache_key".to_owned(),
                serde_json::Value::String(session_id.clone()),
            );
        }
    }
}

/// Create a [`CacheAdapter`] instance for the given provider.
///
/// Returns the provider-specific adapter when one exists
/// (Anthropic, Kimi), or [`NoopCacheAdapter`] for providers
/// that rely on server-side automatic prefix caching.
pub fn for_provider(provider_id: &str) -> Arc<dyn CacheAdapter> {
    match provider_id {
        "anthropic" => Arc::new(AnthropicCacheAdapter),
        "kimi" => Arc::new(KimiCacheAdapter),
        _ => Arc::new(NoopCacheAdapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_session::persistence::ReasoningLevel;
    use serde_json::Map;

    fn make_request() -> InternalRequest {
        InternalRequest {
            model: "test-model".to_owned(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        }
    }

    #[test]
    fn noop_adapter_does_nothing() {
        let mut req = make_request();
        let adapter = NoopCacheAdapter;
        adapter.apply(&mut req);
        assert!(req.system_blocks.is_none());
        assert!(req.extra_body.is_empty());
    }

    #[test]
    fn noop_adapter_name() {
        assert_eq!(NoopCacheAdapter.name(), "noop");
    }

    #[test]
    fn anthropic_adapter_static_only() {
        let mut req = make_request();
        req.system_static = Some("Section A\n\nSection B".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Section A");
        assert!(blocks[0].cache);
        assert_eq!(blocks[1].text, "Section B");
        assert!(blocks[1].cache);
    }

    #[test]
    fn anthropic_adapter_static_and_dynamic() {
        let mut req = make_request();
        req.system_static = Some("Static content".to_owned());
        req.system_dynamic = Some("Dynamic content".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].cache);
        assert!(!blocks[1].cache);
        assert_eq!(blocks[1].text, "Dynamic content");
    }

    #[test]
    fn anthropic_adapter_empty_fields_no_blocks() {
        let mut req = make_request();
        req.system_static = Some("".to_owned());
        AnthropicCacheAdapter.apply(&mut req);
        assert!(req.system_blocks.is_none());
    }

    #[test]
    fn anthropic_adapter_no_fields_no_blocks() {
        let mut req = make_request();
        AnthropicCacheAdapter.apply(&mut req);
        assert!(req.system_blocks.is_none());
    }

    #[test]
    fn anthropic_adapter_name() {
        assert_eq!(AnthropicCacheAdapter.name(), "anthropic");
    }

    #[test]
    fn kimi_adapter_injects_cache_key() {
        let mut req = make_request();
        req.session_id = Some("sess-123".to_owned());
        KimiCacheAdapter.apply(&mut req);

        assert_eq!(
            req.extra_body.get("prompt_cache_key").unwrap(),
            &serde_json::Value::String("sess-123".to_owned())
        );
    }

    #[test]
    fn kimi_adapter_no_session_id_no_inject() {
        let mut req = make_request();
        KimiCacheAdapter.apply(&mut req);
        assert!(req.extra_body.is_empty());
    }

    #[test]
    fn kimi_adapter_name() {
        assert_eq!(KimiCacheAdapter.name(), "kimi");
    }

    #[test]
    fn anthropic_adapter_marks_tools_as_cacheable() {
        use crate::types::ToolDefinition;

        let mut req = make_request();
        req.tools = Some(vec![
            ToolDefinition {
                name: "read_file".to_owned(),
                description: "Read a file from disk".to_owned(),
                input_schema: None,
                cache: false,
            },
            ToolDefinition {
                name: "write_file".to_owned(),
                description: "Write content to a file".to_owned(),
                input_schema: None,
                cache: false,
            },
        ]);
        AnthropicCacheAdapter.apply(&mut req);

        let tools = req.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools[0].cache, "tool should be marked cacheable");
        assert!(tools[1].cache, "tool should be marked cacheable");
    }

    #[test]
    fn anthropic_adapter_no_tools_no_change() {
        let mut req = make_request();
        req.tools = None;
        AnthropicCacheAdapter.apply(&mut req);
        assert!(req.tools.is_none());
    }

    #[test]
    fn anthropic_adapter_empty_tools_no_change() {
        let mut req = make_request();
        req.tools = Some(vec![]);
        AnthropicCacheAdapter.apply(&mut req);
        let tools = req.tools.as_ref().unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn anthropic_adapter_tools_section_in_static() {
        let mut req = make_request();
        req.system_static = Some(
            "## RoleSection\n\n\
                You are a helpful assistant.\n\n\
                ## ToolsSection\n\n\
                ### file_system\n\
                - `read` (dangerous: low)\n\
                - `write` (dangerous: medium)\n\n\
                ### code\n\
                - `edit` (dangerous: medium)\n"
                .to_owned(),
        );
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        // Split by "\n\n" produces 3 non-empty blocks: RoleSection,
        // ToolsSection header, and ToolsSection content.
        assert!(blocks.len() >= 2);
        for block in blocks {
            assert!(block.cache, "block should be cached: {:?}", block.text);
        }
    }

    // ------------------------------------------------------------------
    // for_provider factory function tests
    // ------------------------------------------------------------------

    #[test]
    fn for_provider_anthropic_returns_anthropic_adapter() {
        let adapter = for_provider("anthropic");
        assert_eq!(adapter.name(), "anthropic");
    }

    #[test]
    fn for_provider_kimi_returns_kimi_adapter() {
        let adapter = for_provider("kimi");
        assert_eq!(adapter.name(), "kimi");
    }

    #[test]
    fn for_provider_unknown_returns_noop() {
        for provider_id in ["openai", "deepseek", "mimo", ""] {
            let adapter = for_provider(provider_id);
            assert_eq!(
                adapter.name(),
                "noop",
                "expected noop for provider_id: {provider_id:?}"
            );
        }
    }

    #[test]
    fn for_provider_anthropic_applies_cache_marks() {
        let adapter = for_provider("anthropic");
        let mut req = make_request();
        req.system_static = Some("Static section".to_owned());

        adapter.apply(&mut req);

        let blocks = req
            .system_blocks
            .as_ref()
            .expect("system_blocks should be set");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Static section");
        assert!(blocks[0].cache, "static block should have cache: true");
    }
}
