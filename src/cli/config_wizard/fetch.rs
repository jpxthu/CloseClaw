//! Model fetching logic — delegated to ModelDiscovery

use crate::llm::ModelInfo;

/// Return model list from the knowledge base for the given provider name.
///
/// Delegates to [`crate::llm::ModelDiscovery::knowledge_fallback`].
pub async fn knowledge_fallback(provider_name: &str) -> Vec<ModelInfo> {
    let discovery = crate::llm::ModelDiscovery::new();
    discovery.knowledge_fallback(provider_name)
}
