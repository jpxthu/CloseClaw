//! Model fetching logic — delegated to ModelDiscovery

use closeclaw_llm::DiscoveryResult;

/// Return model list from the knowledge base for the given provider name.
///
/// Delegates to [`closeclaw_llm::ModelDiscovery::knowledge_fallback`].
pub async fn knowledge_fallback(provider_name: &str) -> DiscoveryResult {
    let discovery = closeclaw_llm::ModelDiscovery::new();
    discovery.knowledge_fallback(provider_name)
}
