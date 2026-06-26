//! Unit tests for ModelDiscovery — separated from implementation per CONTRIBUTING.md.

use super::model_cache::ModelCache;
use super::model_discovery::ModelDiscovery;
use super::model_info::{DiscoverySource, ModelInfo};
use super::ProviderModelKnowledge;

/// Helper: create a ModelDiscovery with an isolated temp-dir cache.
fn make_test_discovery(dir: &tempfile::TempDir) -> ModelDiscovery {
    ModelDiscovery {
        cache: ModelCache::with_path(dir.path().join("cache.json")),
        knowledge: ProviderModelKnowledge::new(),
    }
}

fn test_models() -> Vec<ModelInfo> {
    vec![ModelInfo {
        id: "test-model-1".into(),
        name: "Test Model 1".into(),
        context_window: 4096,
        max_tokens: 1024,
        default_temperature: Some(0.7),
        reasoning: false,
        input_types: vec![],
    }]
}

// ── Step 1.4: source attribution + field filling tests ─────────

#[tokio::test]
async fn test_discover_returns_api_source() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = make_test_discovery(&dir);

    let result = discovery
        .discover("minimax", "key", |_| {
            let value = test_models();
            async move { Ok(value) }
        })
        .await;

    assert_eq!(result.source, DiscoverySource::Api);
}

#[tokio::test]
async fn test_cache_hit_returns_cache_source() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = make_test_discovery(&dir);

    discovery
        .cache
        .set("test-provider", "mytoken", test_models());

    let result = discovery
        .discover("test-provider", "mytoken", |_| async {
            unreachable!("fetch should not be called")
        })
        .await;

    assert_eq!(result.source, DiscoverySource::Cache);
}

#[tokio::test]
async fn test_knowledge_fallback_returns_fallback_source() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = make_test_discovery(&dir);

    let result = discovery.knowledge_fallback("minimax");
    assert_eq!(result.source, DiscoverySource::KnowledgeFallback);
}

#[tokio::test]
async fn test_discover_success_path_fills_default_temperature() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = make_test_discovery(&dir);

    // API returns default_temperature: None — knowledge base should fill it.
    let api_models = vec![ModelInfo {
        id: "MiniMax-M2.7".into(),
        name: "MiniMax M2.7".into(),
        context_window: 204_800,
        max_tokens: 131_072,
        default_temperature: None,
        reasoning: false,
        input_types: vec![],
    }];

    let result = discovery
        .discover("minimax", "key", |_| {
            let value = api_models.clone();
            async move { Ok(value) }
        })
        .await;

    let m = &result.models()[0];
    assert_eq!(
        m.default_temperature,
        Some(1.0),
        "knowledge base should fill default_temperature when API is None"
    );
}

#[tokio::test]
async fn test_discover_success_path_fills_input_types() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = make_test_discovery(&dir);

    // API returns empty input_types — knowledge base should fill it.
    let api_models = vec![ModelInfo {
        id: "MiniMax-M2.7".into(),
        name: "MiniMax M2.7".into(),
        context_window: 204_800,
        max_tokens: 131_072,
        default_temperature: Some(0.8),
        reasoning: false,
        input_types: vec![],
    }];

    let result = discovery
        .discover("minimax", "key", |_| {
            let value = api_models.clone();
            async move { Ok(value) }
        })
        .await;

    let m = &result.models()[0];
    assert_eq!(
        m.input_types,
        vec![super::InputType::Text],
        "knowledge base should fill input_types when API returns empty"
    );
}
