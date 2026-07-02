//! Tests for PromptBuilder: priority sorting, cache behaviour, and fallback.

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};
use crate::sections::{
    get_cached_section, invalidate_all_sections, invalidate_section, put_cached_section,
};
use async_trait::async_trait;

const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

// ---------------------------------------------------------------------------
// Mock providers for unit-testing builder logic in isolation
// ---------------------------------------------------------------------------

struct MockProvider {
    name: String,
    priority: u32,
    fragment: Option<PromptFragment>,
    cache_key_val: Option<String>,
}

impl MockProvider {
    fn with_fragment(name: &str, priority: u32, content: &str) -> Self {
        Self {
            name: name.to_string(),
            priority,
            fragment: Some(PromptFragment {
                title: format!("## {}", name),
                section_type: SectionType::Bootstrap,
                content: content.to_string(),
            }),
            cache_key_val: None,
        }
    }

    fn empty(name: &str, priority: u32) -> Self {
        Self {
            name: name.to_string(),
            priority,
            fragment: None,
            cache_key_val: None,
        }
    }

    fn with_cache_key(mut self, key: &str) -> Self {
        self.cache_key_val = Some(key.to_string());
        self
    }
}

#[async_trait]
impl PromptFragmentProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn generate(&self, _ctx: &FragmentContext) -> Option<PromptFragment> {
        self.fragment.clone()
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        self.cache_key_val.clone()
    }
}

// ---------------------------------------------------------------------------
// Priority sorting
// ---------------------------------------------------------------------------

#[test]
fn test_providers_sorted_by_priority() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::with_fragment("third", 30, "c")),
        Box::new(MockProvider::with_fragment("first", 1, "a")),
        Box::new(MockProvider::with_fragment("second", 10, "b")),
    ];
    let mut sorted = providers;
    sorted.sort_by_key(|p| p.priority());

    assert_eq!(sorted[0].name(), "first");
    assert_eq!(sorted[1].name(), "second");
    assert_eq!(sorted[2].name(), "third");
}

#[test]
fn test_providers_with_equal_priority_stable_order() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::with_fragment("a", 5, "content-a")),
        Box::new(MockProvider::with_fragment("b", 5, "content-b")),
    ];
    let mut sorted = providers;
    sorted.sort_by_key(|p| p.priority());
    // Both have priority 5 — order is stable (insertion order preserved by sort_by_key).
    assert_eq!(sorted[0].name(), "a");
    assert_eq!(sorted[1].name(), "b");
}

// ---------------------------------------------------------------------------
// All providers return None → fallback DEFAULT_PROMPT
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_all_providers_none_fallback() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::empty("p1", 1)),
        Box::new(MockProvider::empty("p2", 2)),
        Box::new(MockProvider::empty("p3", 3)),
    ];

    let result = build_from_mocks(providers).await;
    assert_eq!(result, DEFAULT_PROMPT);
}

#[tokio::test]
async fn test_single_provider_none_fallback() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> =
        vec![Box::new(MockProvider::empty("only", 1))];

    let result = build_from_mocks(providers).await;
    assert_eq!(result, DEFAULT_PROMPT);
}

// ---------------------------------------------------------------------------
// Single / multiple providers produce output
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_single_provider_produces_output() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![Box::new(
        MockProvider::with_fragment("tools", 1, "tool content"),
    )];

    let result = build_from_mocks(providers).await;
    assert!(result.contains("## tools"));
    assert!(result.contains("tool content"));
}

#[tokio::test]
async fn test_multiple_providers_concatenated() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::with_fragment("bootstrap", 1, "boot")),
        Box::new(MockProvider::with_fragment("tools", 2, "tool")),
        Box::new(MockProvider::with_fragment("memory", 3, "mem")),
    ];

    let result = build_from_mocks(providers).await;
    let boot_pos = result.find("boot").unwrap();
    let tool_pos = result.find("tool").unwrap();
    let mem_pos = result.find("mem").unwrap();
    assert!(boot_pos < tool_pos);
    assert!(tool_pos < mem_pos);
}

// ---------------------------------------------------------------------------
// Empty provider skipped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_provider_skipped() {
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::empty("empty_one", 1)),
        Box::new(MockProvider::with_fragment("real", 2, "real content")),
    ];

    let result = build_from_mocks(providers).await;
    assert!(!result.contains("empty_one"));
    assert!(result.contains("real content"));
}

// ---------------------------------------------------------------------------
// Cache hit / miss
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cache_hit_skips_generate() {
    invalidate_all_sections();

    // Pre-populate the cache with a known key.
    put_cached_section("mock-cache-key", "cached content".to_string(), None);

    let provider =
        MockProvider::with_fragment("cached", 1, "fresh content").with_cache_key("mock-cache-key");
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![Box::new(provider)];

    let result = build_from_mocks(providers).await;
    // Should use cached content, not the provider's fresh content.
    assert!(result.contains("cached content"));
    assert!(!result.contains("fresh content"));

    invalidate_all_sections();
}

#[tokio::test]
async fn test_cache_miss_calls_generate() {
    invalidate_all_sections();

    let provider =
        MockProvider::with_fragment("fresh", 1, "generated content").with_cache_key("fresh-key");
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![Box::new(provider)];

    let result = build_from_mocks(providers).await;
    // Provider was called and generated fresh content.
    assert!(result.contains("generated content"));

    invalidate_all_sections();
}

#[tokio::test]
async fn test_cache_invalidation_triggers_regenerate() {
    invalidate_all_sections();

    // Cache with old content.
    put_cached_section("regen-key", "old content".to_string(), None);

    let provider =
        MockProvider::with_fragment("regen", 1, "new content").with_cache_key("regen-key");
    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![Box::new(provider)];

    // Before invalidation → cache hit → old content.
    let result = build_from_mocks(providers).await;
    assert!(result.contains("old content"));

    // Invalidate → cache miss → provider generates new content.
    invalidate_section("regen-key");
    let provider2 =
        MockProvider::with_fragment("regen", 1, "new content").with_cache_key("regen-key");
    let providers2: Vec<Box<dyn PromptFragmentProvider>> = vec![Box::new(provider2)];
    let result2 = build_from_mocks(providers2).await;
    assert!(result2.contains("new content"));
    assert!(!result2.contains("old content"));

    invalidate_all_sections();
}

// ---------------------------------------------------------------------------
// Mixed: some providers empty, some cached
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mixed_empty_and_cached_providers() {
    invalidate_all_sections();

    put_cached_section("mix-cache", "cached data".to_string(), None);

    let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
        Box::new(MockProvider::empty("empty1", 1)),
        Box::new(
            MockProvider::with_fragment("cached", 2, "fresh data").with_cache_key("mix-cache"),
        ),
        Box::new(MockProvider::empty("empty2", 3)),
    ];

    let result = build_from_mocks(providers).await;
    assert!(result.contains("cached data"));
    assert!(!result.contains("fresh data"));

    invalidate_all_sections();
}

// ---------------------------------------------------------------------------
// Helper: build prompt from a list of mock providers
// ---------------------------------------------------------------------------

/// Assemble a prompt string from mock providers using the same logic as
/// `PromptBuilder::build` but without real registries.
async fn build_from_mocks(mut providers: Vec<Box<dyn PromptFragmentProvider>>) -> String {
    providers.sort_by_key(|p| p.priority());

    let mut fragments: Vec<String> = Vec::new();

    for provider in &providers {
        if let Some(key) = provider.cache_key(&FragmentContext::default()) {
            if let Some(cached) = get_cached_section(&key, None) {
                fragments.push(cached);
                continue;
            }
        }

        if let Some(fragment) = provider.generate(&FragmentContext::default()).await {
            let rendered = format!("{}\n{}\n", fragment.title, fragment.content);
            if let Some(key) = provider.cache_key(&FragmentContext::default()) {
                put_cached_section(&key, rendered.clone(), None);
            }
            fragments.push(rendered);
        }
    }

    if fragments.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        fragments.join("\n")
    }
}
