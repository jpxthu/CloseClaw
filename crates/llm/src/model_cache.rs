//! LLM Model Discovery Cache
//!
//! Caches the list of available models per (provider, token) pair to avoid
//! calling the API on every startup. Entries expire after a configurable TTL.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model_info::ModelInfo;

/// A single cache entry storing the list of models and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Unix timestamp (seconds) when this entry was fetched.
    pub fetched_at: i64,
    /// TTL in seconds for this entry.
    pub ttl_secs: u64,
    /// Cached model list.
    pub models: Vec<ModelInfo>,
}

impl CacheEntry {
    /// Returns true if this entry has expired based on current time.
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        now - self.fetched_at >= self.ttl_secs as i64
    }
}

/// Cache key derived from provider and token prefix.
pub struct CacheKey;

impl CacheKey {
    /// Extract the first 4 characters of a token as its prefix.
    pub fn token_prefix(token: &str) -> String {
        token.chars().take(4).collect()
    }

    /// Compute a cache key by hashing `provider:token_prefix`.
    /// Result is a SHA256 hex string.
    pub fn compute(provider: &str, token_prefix: &str) -> String {
        let input = format!("{provider}:{token_prefix}");
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)
    }
}

/// In-memory cache backed by a JSON file on disk.
pub struct ModelCache {
    persist_path: std::path::PathBuf,
}

impl ModelCache {
    /// Construct a new ModelCache.
    ///
    /// Path resolution (first match wins):
    /// 1. `MODEL_CACHE_FILE` environment variable
    /// 2. `~/.closeclaw/model_cache.json`
    /// 3. `model_cache.json` in current directory
    pub fn new() -> Self {
        let persist_path = std::env::var("MODEL_CACHE_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| {
                        std::path::PathBuf::from(h)
                            .join(".closeclaw")
                            .join("model_cache.json")
                    })
                    .unwrap_or_else(|_| std::path::PathBuf::from("model_cache.json"))
            });

        Self { persist_path }
    }

    /// Construct a `ModelCache` with an explicit file path.
    #[cfg(test)]
    pub(crate) fn with_path(persist_path: std::path::PathBuf) -> Self {
        Self { persist_path }
    }

    /// Retrieve cached models for the given provider + token.
    ///
    /// Returns `None` if:
    /// - the cache file does not exist
    /// - the file cannot be parsed as valid JSON
    /// - the key is not present
    /// - the entry has expired
    pub fn get(&self, provider: &str, token: &str) -> Option<Vec<ModelInfo>> {
        let key = CacheKey::compute(provider, &CacheKey::token_prefix(token));

        let data = std::fs::read_to_string(&self.persist_path).ok()?;
        let cache: std::collections::HashMap<String, CacheEntry> =
            serde_json::from_str(&data).ok()?;

        let entry = cache.get(&key)?;

        if entry.is_expired() {
            return None;
        }

        Some(entry.models.clone())
    }

    /// Persist the given model list under the given provider + token key.
    ///
    /// If the cache file exists it is loaded first to preserve other entries.
    pub fn set(&self, provider: &str, token: &str, models: Vec<ModelInfo>) {
        let key = CacheKey::compute(provider, &CacheKey::token_prefix(token));
        let now = chrono::Utc::now().timestamp();

        // Load existing cache or start empty
        let mut cache: std::collections::HashMap<String, CacheEntry> =
            std::fs::read_to_string(&self.persist_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default();

        cache.insert(
            key,
            CacheEntry {
                fetched_at: now,
                ttl_secs: 3600,
                models,
            },
        );

        if let Some(parent) = self.persist_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(&cache) {
            let _ = std::fs::write(&self.persist_path, data);
        }
    }

    /// Remove all expired entries from the cache file.
    pub fn clear_expired(&self) {
        let data = match std::fs::read_to_string(&self.persist_path) {
            Ok(d) => d,
            Err(_) => return,
        };
        let mut cache: std::collections::HashMap<String, CacheEntry> =
            match serde_json::from_str(&data) {
                Ok(m) => m,
                Err(_) => return,
            };

        cache.retain(|_, entry| !entry.is_expired());

        if let Some(parent) = self.persist_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(&cache) {
            let _ = std::fs::write(&self.persist_path, data);
        }
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = CacheKey::compute("minimax", "TMA-");
        let k2 = CacheKey::compute("minimax", "TMA-");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_cache_key_different_inputs_different_keys() {
        let k1 = CacheKey::compute("minimax", "TMA-");
        let k2 = CacheKey::compute("glm", "TMA-");
        let k3 = CacheKey::compute("minimax", "GLM-");
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_token_prefix() {
        assert_eq!(CacheKey::token_prefix("TMA-abc123"), "TMA-");
        assert_eq!(CacheKey::token_prefix("ab"), "ab");
        assert_eq!(CacheKey::token_prefix(""), "");
    }

    #[test]
    fn test_cache_entry_is_expired() {
        let past = chrono::Utc::now().timestamp() - 7200;
        let entry = CacheEntry {
            fetched_at: past,
            ttl_secs: 3600,
            models: vec![],
        };
        assert!(entry.is_expired());

        let recent = chrono::Utc::now().timestamp() - 100;
        let entry2 = CacheEntry {
            fetched_at: recent,
            ttl_secs: 3600,
            models: vec![],
        };
        assert!(!entry2.is_expired());
    }

    #[test]
    fn test_get_file_not_exist() {
        let cache = ModelCache {
            persist_path: std::path::PathBuf::from("/nonexistent/path/cache.json"),
        };
        assert!(cache.get("minimax", "token").is_none());
    }

    #[test]
    fn test_set_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = ModelCache { persist_path: path };

        let models = vec![ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            context_window: 4096,
            max_tokens: 1024,
            default_temperature: Some(0.7),
            reasoning: false,
            input_types: vec![],
        }];

        cache.set("provider", "token123", models.clone());
        let retrieved = cache.get("provider", "token123");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].id, "test-model");
    }

    #[test]
    fn test_get_key_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = ModelCache { persist_path: path };

        cache.set("provider", "token123", vec![]);
        assert!(cache.get("other-provider", "token123").is_none());
    }

    #[test]
    fn test_corrupted_json_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, "not valid json {{{").unwrap();

        let cache = ModelCache { persist_path: path };
        assert!(cache.get("minimax", "token").is_none());
    }
}
