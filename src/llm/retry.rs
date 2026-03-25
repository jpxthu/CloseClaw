//! LLM Retry & Cooldown Management
//!
//! Implements exponential backoff retry and per-(provider, model) cooldown tracking.

use crate::llm::ErrorKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Maximum number of retry attempts for transient errors
pub const MAX_TRANSIENT_RETRIES: u32 = 3;
/// Maximum number of retry attempts for unknown errors
pub const MAX_UNKNOWN_RETRIES: u32 = 1;

/// Base delay for transient error backoff: 1 minute, doubled each attempt
pub const TRANSIENT_BASE_DELAY: Duration = Duration::from_secs(60);
/// Cap for transient error backoff: 1 hour
pub const TRANSIENT_MAX_DELAY: Duration = Duration::from_secs(3600);
/// Cap for billing error backoff: 24 hours
pub const BILLING_MAX_DELAY: Duration = Duration::from_secs(86400);

/// Cooldown entry for a (provider, model) pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CooldownEntry {
    /// Consecutive failure count
    pub attempts: u32,
    /// When the cooldown expires (UTC ISO timestamp)
    pub cooldown_until: String,
    /// Reason for cooldown: "transient" | "billing" | "auth"
    pub reason: String,
}

/// Manages cooldowns for (provider, model) pairs
pub struct CooldownManager {
    /// In-memory cooldowns: key = "provider/model"
    cooldowns: RwLock<HashMap<String, CooldownEntry>>,
    /// Path to persist cooldowns to disk
    persist_path: std::path::PathBuf,
}

impl Default for CooldownManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CooldownManager {
    pub fn new() -> Self {
        let persist_path = std::env::var("LLM_COOLDOWN_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".closeclaw/llm_cooldowns.json"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("llm_cooldowns.json"))
            });

        Self {
            cooldowns: RwLock::new(HashMap::new()),
            persist_path,
        }
    }

    /// Returns true if the given provider/model is in cooldown
    pub async fn is_in_cooldown(&self, provider: &str, model: &str) -> bool {
        let cooldowns = self.cooldowns.read().await;
        if let Some(entry) = cooldowns.get(&Self::key(provider, model)) {
            // Parse the ISO timestamp and check if still valid
            if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(&entry.cooldown_until) {
                return chrono::Utc::now() < expiry;
            }
        }
        false
    }

    /// Record a failure and enter/update cooldown
    pub async fn record_failure(&self, provider: &str, model: &str, kind: ErrorKind) {
        let key = Self::key(provider, model);
        let mut cooldowns = self.cooldowns.write().await;

        let entry = cooldowns
            .entry(key.clone())
            .or_insert_with(|| CooldownEntry {
                attempts: 0,
                cooldown_until: String::new(),
                reason: String::new(),
            });

        entry.attempts += 1;

        let delay = match kind {
            ErrorKind::Transient => {
                let secs = TRANSIENT_BASE_DELAY.as_secs() * 2u64.pow(entry.attempts.min(5));
                Duration::from_secs(secs).min(TRANSIENT_MAX_DELAY)
            }
            ErrorKind::Billing => {
                let secs = 18000u64 * 2u64.pow(entry.attempts.min(3)); // 5h base
                Duration::from_secs(secs).min(BILLING_MAX_DELAY)
            }
            ErrorKind::Auth => {
                // Auth errors get a long cooldown since credentials are invalid
                Duration::from_secs(3600) // 1h
            }
            ErrorKind::InvalidRequest => {
                // No cooldown needed, just don't retry
                return;
            }
            ErrorKind::Unknown => {
                // Short cooldown for unknown errors
                Duration::from_secs(30)
            }
        };

        let expiry = chrono::Utc::now()
            + chrono::Duration::from_std(delay)
                .unwrap_or_else(|_| chrono::Duration::from_std(TRANSIENT_MAX_DELAY).unwrap());
        entry.cooldown_until = expiry.to_rfc3339();
        entry.reason = format!("{:?}", kind).to_lowercase();

        drop(cooldowns);
        self.save().await;
    }

    /// Reset cooldown on successful call
    pub async fn record_success(&self, provider: &str, model: &str) {
        let key = Self::key(provider, model);
        let mut cooldowns = self.cooldowns.write().await;
        cooldowns.remove(&key);
        drop(cooldowns);
        self.save().await;
    }

    /// Load persisted cooldowns from disk (sync version for use in sync constructors).
    ///
    /// Uses a one-shot runtime to block on async file I/O. Safe to call from sync
    /// startup code (where no runtime is running yet). Skipped when called from
    /// within a running runtime (e.g., in tests) since tests don't need persisted cooldowns.
    #[allow(dead_code)]
    pub fn load_sync(&self) {
        // Only load when NO Tokio runtime is running (startup context).
        // When a runtime IS running (tests, async contexts), skip — the in-memory
        // cooldowns are empty anyway in those cases.
        if tokio::runtime::Handle::try_current().is_ok() {
            return;
        }

        if !self.persist_path.exists() {
            return;
        }
        let data = match std::fs::read_to_string(&self.persist_path) {
            Ok(d) => d,
            Err(_) => return,
        };
        let loaded: HashMap<String, CooldownEntry> = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(_) => return,
        };
        let now = chrono::Utc::now();
        let valid: HashMap<String, CooldownEntry> = loaded
            .into_iter()
            .filter(|(_, entry)| {
                if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(&entry.cooldown_until) {
                    now < expiry
                } else {
                    false
                }
            })
            .collect();
        if valid.is_empty() {
            return;
        }

        // Use a one-shot runtime to run the async load() to completion
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async {
            let mut cooldowns = self.cooldowns.write().await;
            cooldowns.extend(valid);
        });
    }

    /// Load persisted cooldowns from disk
    #[allow(dead_code)]
    pub async fn load(&self) {
        if !self.persist_path.exists() {
            return;
        }
        let data = match tokio::fs::read_to_string(&self.persist_path).await {
            Ok(d) => d,
            Err(_) => return,
        };
        let loaded: HashMap<String, CooldownEntry> = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(_) => return,
        };
        // Filter out expired entries
        let now = chrono::Utc::now();
        let valid: HashMap<String, CooldownEntry> = loaded
            .into_iter()
            .filter(|(_, entry)| {
                if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(&entry.cooldown_until) {
                    now < expiry
                } else {
                    false
                }
            })
            .collect();
        let mut cooldowns = self.cooldowns.write().await;
        cooldowns.extend(valid);
    }

    /// Save cooldowns to disk
    async fn save(&self) {
        use tokio::fs;
        if let Some(parent) = self.persist_path.parent() {
            let _ = fs::create_dir_all(parent).await;
        }
        let cooldowns = self.cooldowns.read().await;
        if let Ok(data) = serde_json::to_string_pretty(&*cooldowns) {
            let _ = fs::write(&self.persist_path, data).await;
        }
    }

    fn key(provider: &str, model: &str) -> String {
        format!("{}/{}", provider, model)
    }
}

/// Calculate exponential backoff delay with jitter
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let exponential = base.as_secs() * 2u64.saturating_pow(attempt.saturating_sub(1));
    // Simple deterministic jitter (0-10% of delay)
    let jitter_factor = ((attempt * 7 + 3) % 10) as f64 / 100.0;
    let jitter = (exponential as f64 * jitter_factor).min(max.as_secs() as f64);
    Duration::from_secs((exponential as f64 + jitter) as u64).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cooldown_record_failure_transient() {
        let manager = CooldownManager::new();
        manager
            .record_failure("minimax", "MiniMax-M2.7", ErrorKind::Transient)
            .await;

        assert!(manager.is_in_cooldown("minimax", "MiniMax-M2.7").await);
        // Non-cooldown model should not be affected
        assert!(!manager.is_in_cooldown("minimax", "MiniMax-M2.5").await);
    }

    #[tokio::test]
    async fn test_cooldown_no_cooldown_for_invalid_request() {
        let manager = CooldownManager::new();
        manager
            .record_failure("minimax", "MiniMax-M2.7", ErrorKind::InvalidRequest)
            .await;

        // InvalidRequest should not set cooldown
        assert!(!manager.is_in_cooldown("minimax", "MiniMax-M2.7").await);
    }

    #[tokio::test]
    async fn test_cooldown_success_clears() {
        let manager = CooldownManager::new();
        manager
            .record_failure("minimax", "MiniMax-M2.7", ErrorKind::Transient)
            .await;
        assert!(manager.is_in_cooldown("minimax", "MiniMax-M2.7").await);

        manager.record_success("minimax", "MiniMax-M2.7").await;
        assert!(!manager.is_in_cooldown("minimax", "MiniMax-M2.7").await);
    }

    #[test]
    fn test_backoff_delay_increases() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(100);

        let d1 = backoff_delay(1, base, max);
        let d2 = backoff_delay(2, base, max);
        let d3 = backoff_delay(3, base, max);

        assert!(d2 >= d1);
        assert!(d3 >= d2);
        assert!(d3 <= max);
    }

    #[test]
    fn test_error_kind_classification() {
        use crate::llm::LLMError;

        assert_eq!(LLMError::AuthFailed("bad".into()).kind(), ErrorKind::Auth);
        assert_eq!(LLMError::RateLimitExceeded.kind(), ErrorKind::Transient);
        assert_eq!(
            LLMError::InvalidRequest("bad".into()).kind(),
            ErrorKind::InvalidRequest
        );
        assert_eq!(
            LLMError::NetworkError("timeout".into()).kind(),
            ErrorKind::Transient
        );
    }
}
