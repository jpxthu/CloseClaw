use super::format_discovery_source_suffix;
use closeclaw_llm::DiscoverySource;

#[test]
fn test_knowledge_fallback_suffix() {
    assert_eq!(
        format_discovery_source_suffix(DiscoverySource::KnowledgeFallback),
        " (via knowledge base fallback)"
    );
}

#[test]
fn test_cache_suffix_empty() {
    assert_eq!(format_discovery_source_suffix(DiscoverySource::Cache), "");
}

#[test]
fn test_api_suffix_empty() {
    assert_eq!(format_discovery_source_suffix(DiscoverySource::Api), "");
}
