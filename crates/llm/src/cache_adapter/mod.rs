//! Cache adapter layer for LLM provider-specific prompt caching strategies.
//!
//! Different LLM providers expose different caching mechanisms. This module
//! provides a [`CacheAdapter`] trait and implementations for each provider's
//! caching strategy. The adapter runs *before* the Plugin Pipeline, acting as
//! an independent pre-processing step on the request hot path.
//!
//! Provider mapping (see `docs/design/llm/cache-adapter.md`):
//! - Anthropic: explicit `cache_control` on static system blocks
//! - MiniMax: Anthropic-compatible, reuses `AnthropicCacheAdapter`
//! - All others (OpenAI, DeepSeek, Kimi, …): no-op passthrough

use std::sync::Arc;

use crate::types::{InternalRequest, SystemBlock};

/// Adapter trait for provider-specific prompt caching strategies.
///
/// Implementations transform an [`InternalRequest`] in place, injecting
/// provider-specific cache parameters (e.g., Anthropic `cache_control`).
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

        // Dynamic area (includes merged appends) as a single non-cacheable
        // block. Appends are merged into system_dynamic by the builder
        // (see design doc kv-cache.md §数据流: two-field contract).
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

/// Create a [`CacheAdapter`] instance for the given provider.
///
/// Returns the provider-specific adapter when one exists
/// (Anthropic, MiniMax), or [`NoopCacheAdapter`] for providers
/// that rely on server-side automatic prefix caching.
pub fn for_provider(provider_id: &str) -> Arc<dyn CacheAdapter> {
    match provider_id {
        "anthropic" => Arc::new(AnthropicCacheAdapter),
        // MiniMax uses an Anthropic-compatible interface that supports explicit
        // prefix caching via `cache_control` on static system blocks (see
        // docs/design/llm/providers/minimax.md "缓存机制"). Reuse the
        // AnthropicCacheAdapter strategy — caching is chosen by provider API
        // capability, not protocol format (see cache-adapter.md).
        "minimax" => Arc::new(AnthropicCacheAdapter),

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
    fn for_provider_kimi_returns_noop() {
        let adapter = for_provider("kimi");
        assert_eq!(adapter.name(), "noop");
    }

    #[test]
    fn for_provider_unknown_returns_noop() {
        for provider_id in ["openai", "deepseek", "mimo", "kimi", ""] {
            let adapter = for_provider(provider_id);
            assert_eq!(
                adapter.name(),
                "noop",
                "expected noop for provider_id: {provider_id:?}"
            );
        }
    }

    #[test]
    fn for_provider_minimax_returns_anthropic_adapter() {
        let adapter = for_provider("minimax");
        assert_eq!(adapter.name(), "anthropic");
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

    // ------------------------------------------------------------------
    // Noop pass-through boundary tests
    // ------------------------------------------------------------------

    /// Verify that NoopCacheAdapter preserves existing `extra_body`
    /// contents when a Kimi-like request carries `session_id` and
    /// non-empty `extra_body`. The adapter must neither inject
    /// `prompt_cache_key` nor clear/modify the map — it is a strict
    /// passthrough (see docs/design/llm/cache-adapter.md § 其他供应商).
    #[test]
    fn noop_preserves_extra_body_with_session_id() {
        let mut req = make_request();
        req.session_id = Some("sess-kimi-123".to_owned());
        req.extra_body
            .insert("temperature".to_owned(), serde_json::json!(0.7));
        req.extra_body
            .insert("top_p".to_owned(), serde_json::json!(0.9));

        let adapter = NoopCacheAdapter;
        adapter.apply(&mut req);

        // extra_body must be unchanged
        assert_eq!(req.extra_body.len(), 2);
        assert_eq!(req.extra_body["temperature"], serde_json::json!(0.7));
        assert_eq!(req.extra_body["top_p"], serde_json::json!(0.9));
        // No prompt_cache_key injected
        assert!(!req.extra_body.contains_key("prompt_cache_key"));
        // session_id is untouched (adapter does not read it)
        assert_eq!(req.session_id.as_deref(), Some("sess-kimi-123"));
    }

    /// Verify that NoopCacheAdapter clears nothing and adds nothing
    /// when session_id is None — an empty extra_body stays empty.
    #[test]
    fn noop_empty_request_unchanged() {
        let mut req = make_request();
        req.session_id = None;

        let adapter = NoopCacheAdapter;
        adapter.apply(&mut req);

        assert!(req.extra_body.is_empty());
        assert!(req.session_id.is_none());
        assert!(req.system_blocks.is_none());
    }

    // ------------------------------------------------------------------
    // Factory exhaustive mapping tests
    // ------------------------------------------------------------------

    /// Exhaustive mapping: every provider listed as "noop" in the
    /// design doc (docs/design/llm/cache-adapter.md § 其他供应商)
    /// must return NoopCacheAdapter. This covers the full set of
    /// known providers plus the empty-string edge case.
    #[test]
    fn for_provider_noop_providers_exhaustive() {
        for provider_id in [
            "openai",
            "deepseek",
            "mimo",
            "kimi",
            "glm",
            "volcengine",
            "",
        ] {
            let adapter = for_provider(provider_id);
            assert_eq!(
                adapter.name(),
                "noop",
                "expected noop for provider_id: {provider_id:?}"
            );
        }
    }

    /// Exhaustive mapping: providers that use explicit prefix caching
    /// (docs/design/llm/cache-adapter.md § Anthropic 适配 + § MiniMax)
    /// must return AnthropicCacheAdapter.
    #[test]
    fn for_provider_anthropic_providers_exhaustive() {
        for provider_id in ["anthropic", "minimax"] {
            let adapter = for_provider(provider_id);
            assert_eq!(
                adapter.name(),
                "anthropic",
                "expected anthropic for provider_id: {provider_id:?}"
            );
        }
    }

    // ------------------------------------------------------------------
    // Appends partition tests
    // ------------------------------------------------------------------

    /// Appends are appended after dynamic content with cache: false.
    // ------------------------------------------------------------------
    // Two-partition model (appends merged into dynamic)
    // ------------------------------------------------------------------

    /// Static + dynamic-with-appends: appends are merged into the dynamic
    /// field by the builder; adapter produces two blocks.
    #[test]
    fn anthropic_adapter_static_and_dynamic_with_appends() {
        let mut req = make_request();
        req.system_static = Some("Static".to_owned());
        // Builder merges appends into dynamic field
        req.system_dynamic = Some("Dynamic\n\n## Append\n1. Appended text".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Static");
        assert!(blocks[0].cache);
        assert_eq!(blocks[1].text, "Dynamic\n\n## Append\n1. Appended text");
        assert!(!blocks[1].cache);
    }

    /// Dynamic-only with appends merged: single non-cacheable block.
    #[test]
    fn anthropic_adapter_dynamic_only_with_appends_merged() {
        let mut req = make_request();
        // Builder puts appends into system_dynamic
        req.system_dynamic = Some("Dynamic\n\n## Append\n1. Appended text".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Dynamic\n\n## Append\n1. Appended text");
        assert!(!blocks[0].cache);
    }

    /// Appends alone in dynamic field (no static): single non-cacheable block.
    #[test]
    fn anthropic_adapter_appends_in_dynamic_only() {
        let mut req = make_request();
        // When dynamic layer is empty, builder sets dynamic to only append段
        req.system_dynamic = Some("## Append\n1. Append content".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "## Append\n1. Append content");
        assert!(!blocks[0].cache);
    }

    /// Two-partition model: only static + dynamic blocks, no appends partition.
    #[test]
    fn anthropic_adapter_two_partition_no_separate_appends() {
        let mut req = make_request();
        req.system_static = Some("Static".to_owned());
        req.system_dynamic = Some("Dynamic".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        // Only two blocks: static + dynamic; appends are merged into dynamic
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Static");
        assert!(blocks[0].cache);
        assert_eq!(blocks[1].text, "Dynamic");
        assert!(!blocks[1].cache);
    }

    // ------------------------------------------------------------------
    // Regression — static-only + dynamic with merged appends
    // ------------------------------------------------------------------

    /// Static-only + dynamic-with-appends (no appends in static): the dynamic
    /// field contains merged appends, producing a single non-cacheable block.
    #[test]
    fn anthropic_adapter_static_only_with_dynamic_appends() {
        let mut req = make_request();
        req.system_static = Some("Static content".to_owned());
        // Builder merges appends into system_dynamic
        req.system_dynamic = Some("Dynamic\n\n## Append\n1. Append text".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Static content");
        assert!(blocks[0].cache, "static block should be cached");
        assert_eq!(blocks[1].text, "Dynamic\n\n## Append\n1. Append text");
        assert!(!blocks[1].cache, "dynamic block should NOT be cached");
    }

    /// Dynamic-only + appends merged into dynamic: single non-cacheable block.
    #[test]
    fn anthropic_adapter_dynamic_only_with_appends_merged_regression() {
        let mut req = make_request();
        // Builder merges appends into system_dynamic
        req.system_dynamic = Some("Dynamic content\n\n## Append\n1. Append text".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text,
            "Dynamic content\n\n## Append\n1. Append text"
        );
        assert!(!blocks[0].cache, "dynamic block should NOT be cached");
    }

    /// Static + dynamic (with merged appends): two partitions.
    #[test]
    fn anthropic_adapter_two_partition_order_and_marks() {
        let mut req = make_request();
        req.system_static = Some("Static".to_owned());
        req.system_dynamic = Some("Dynamic\n\n## Append\n1. Appends".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Static");
        assert!(blocks[0].cache);
        assert_eq!(blocks[1].text, "Dynamic\n\n## Append\n1. Appends");
        assert!(!blocks[1].cache);
    }

    // ------------------------------------------------------------------
    // Merge boundary cache adapter tests
    // ------------------------------------------------------------------

    /// Static zone split into multiple blocks by double-newline: all blocks
    /// must have cache: true (each section is independently cacheable).
    #[test]
    fn anthropic_adapter_static_multi_blocks_all_cached() {
        let mut req = make_request();
        req.system_static = Some("Section A\n\nSection B\n\nSection C".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "Section A");
        assert!(blocks[0].cache, "Section A should be cached");
        assert_eq!(blocks[1].text, "Section B");
        assert!(blocks[1].cache, "Section B should be cached");
        assert_eq!(blocks[2].text, "Section C");
        assert!(blocks[2].cache, "Section C should be cached");
    }

    /// Dynamic zone with merged appends is a single non-cacheable block.
    /// The entire dynamic+append text is wrapped as one SystemBlock with
    /// cache: false — appends do not create separate blocks.
    #[test]
    fn anthropic_adapter_dynamic_with_appends_single_block_no_cache() {
        let mut req = make_request();
        req.system_static = Some("Static".to_owned());
        req.system_dynamic =
            Some("Dynamic content\n\n## Append\n[0] item A\n[1] item B".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 2);
        // Static block cached
        assert!(blocks[0].cache);
        // Dynamic block (with merged appends) NOT cached
        assert!(!blocks[1].cache);
        assert_eq!(
            blocks[1].text,
            "Dynamic content\n\n## Append\n[0] item A\n[1] item B"
        );
    }

    /// Appends in the dynamic field do NOT form independent blocks.
    /// The entire dynamic text (including ## Append heading and entries)
    /// is a single non-cacheable block.
    #[test]
    fn anthropic_adapter_appends_not_independent_block() {
        let mut req = make_request();
        req.system_static = Some("Static".to_owned());
        // Dynamic field contains merged appends (two ## sections separated
        // by double newline — but adapter must NOT split dynamic by \n\n)
        req.system_dynamic = Some("Channel Context\nchat: test\n\n## Append\n[0] note".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        // Only 2 blocks: static + dynamic (NOT 3)
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].cache);
        assert!(!blocks[1].cache);
        assert_eq!(
            blocks[1].text,
            "Channel Context\nchat: test\n\n## Append\n[0] note"
        );
    }

    /// Static multi-block + dynamic with appends: static blocks cached,
    /// single dynamic block not cached.
    #[test]
    fn anthropic_adapter_static_multi_dynamic_with_appends() {
        let mut req = make_request();
        req.system_static = Some("Part A\n\nPart B".to_owned());
        req.system_dynamic = Some("Dynamic\n\n## Append\n[0] item".to_owned());
        AnthropicCacheAdapter.apply(&mut req);

        let blocks = req.system_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 3);
        assert!(blocks[0].cache, "Part A should be cached");
        assert!(blocks[1].cache, "Part B should be cached");
        assert!(!blocks[2].cache, "Dynamic block should NOT be cached");
        assert_eq!(blocks[2].text, "Dynamic\n\n## Append\n[0] item");
    }
}
