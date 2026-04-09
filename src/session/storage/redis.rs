//! Redis storage backend for session persistence
//!
//! This backend stores checkpoints in Redis with TTL support.
//! Suitable for production deployments.

use crate::session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use async_trait::async_trait;
use redis::AsyncCommands;
use redis;

/// Redis storage backend
#[derive(Debug, Clone)]
pub struct RedisStorage {
    client: redis::Client,
    key_prefix: String,
}

impl RedisStorage {
    /// Create a new RedisStorage instance
    ///
    /// # Errors
    /// Returns `PersistenceError::Redis` if the Redis URL is invalid.
    pub fn new(redis_url: &str, key_prefix: impl Into<String>) -> Result<Self, PersistenceError> {
        let client =
            redis::Client::open(redis_url).map_err(|e| PersistenceError::Redis(e.to_string()))?;
        Ok(Self {
            client,
            key_prefix: key_prefix.into(),
        })
    }

    fn make_key(&self, session_id: &str) -> String {
        format!("{}:{}", self.key_prefix, session_id)
    }

    /// Returns the key prefix used for this storage
    pub fn key_prefix(&self) -> &str {
        &self.key_prefix
    }
}

#[async_trait]
impl PersistenceService for RedisStorage {
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(&checkpoint.session_id);
        let value = serde_json::to_string(checkpoint)?;

        // Set TTL (default 7 days = 604800 seconds)
        let ttl = if checkpoint.ttl_seconds > 0 {
            checkpoint.ttl_seconds
        } else {
            604800
        };

        conn.set_ex::<_, _, ()>(&key, &value, ttl)
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(session_id);
        let value: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        match value {
            Some(v) => {
                let checkpoint: SessionCheckpoint = serde_json::from_str(&v)?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(session_id);
        conn.del::<_, ()>(&key)
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let pattern = format!("{}:*", self.key_prefix);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        // Extract session_id (remove prefix)
        let session_ids = keys
            .iter()
            .map(|k| {
                k.strip_prefix(&format!("{}:", self.key_prefix))
                    .unwrap_or(k)
                    .to_string()
            })
            .collect();

        Ok(session_ids)
    }
}

#[cfg(test)]
mod tests {
    // Integration tests for RedisStorage require a running Redis instance.
    // Run with: cargo test redis_storage -- --ignored
    //
    // #[tokio::test]
    // #[ignore]
    // async fn test_redis_storage_integration() {
    //     let storage = RedisStorage::new("redis://localhost:6379", "test")
    //         .expect("Failed to create RedisStorage");
    //     // ... integration tests
    // }

    use super::*;

    #[test]
    fn test_redis_storage_make_key() {
        let storage = RedisStorage::new("redis://localhost:6379", "checkpoint")
            .expect("Failed to create RedisStorage");

        assert_eq!(storage.make_key("session123"), "checkpoint:session123");
    }

    #[test]
    fn test_redis_storage_make_key_custom_prefix() {
        let storage =
            RedisStorage::new("redis://localhost:6379", "custom_prefix").expect("Failed to create RedisStorage");

        assert_eq!(storage.make_key("abc"), "custom_prefix:abc");
    }

    #[test]
    fn test_redis_storage_key_prefix() {
        let storage =
            RedisStorage::new("redis://localhost:6379", "my_prefix").expect("Failed to create RedisStorage");

        assert_eq!(storage.key_prefix(), "my_prefix");
    }

    #[test]
    fn test_redis_storage_invalid_url() {
        let result = RedisStorage::new("redis://invalid:99999", "test");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PersistenceError::Redis(_)));
    }
}
