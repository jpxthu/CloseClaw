//! KV cache prefix simulation module.
//!
//! Simulates real provider prefix-cache lifecycle (write → hit → break →
//! expired) via a deterministic state machine. See
//! `docs/design/fake_llm/kv-cache-simulation.md`.
//!
//! Two modes are supported (explicit override takes priority):
//! - **Auto simulation** (default): state machine infers cache fields from
//!   request prefix stability.
//! - **Explicit injection**: scenario declares exact cache field values;
//!   state machine records but does not override.

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// FNV-1a deterministic hasher (no random seed).
// ---------------------------------------------------------------------------

struct FnvHasher(u64);

impl FnvHasher {
    fn new() -> Self {
        Self(0xcbf29ce484222325) // FNV offset basis
    }

    fn write_u8(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3); // FNV prime
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u8(b);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

use crate::scenario::types::MessageEntry;

/// Default TTL for cache entries (5 minutes, matching real providers).
const DEFAULT_TTL_SECS: u64 = 300;

/// Cache state machine states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CacheState {
    /// No prior requests seen.
    #[default]
    Empty,
    /// First request for this prefix — cache is being written.
    Writing,
    /// Prefix matched cached state — cache hit.
    Hit,
    /// TTL expired — cache invalidated, next request rewrites.
    Expired,
}

/// Result of KV cache simulation for a single request.
#[derive(Debug, Clone, Default)]
pub struct CacheResult {
    /// Cache hit (read) tokens, if applicable.
    pub cache_hit_tokens: Option<u32>,
    /// Cache write (creation) tokens, if applicable.
    pub cache_write_tokens: Option<u32>,
    /// Current cache state after this request.
    pub state: CacheState,
}

/// KV cache prefix simulation state machine.
///
/// Tracks the last prefix fingerprint and timestamp, computes deterministic
/// cache hit/write token counts based on state transitions.
pub struct KvCacheSimulator {
    last_fingerprint: Option<u64>,
    last_timestamp: Option<Instant>,
    ttl: Duration,
}

impl Default for KvCacheSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl KvCacheSimulator {
    /// Create a simulator with default TTL (5 minutes).
    pub fn new() -> Self {
        Self {
            last_fingerprint: None,
            last_timestamp: None,
            ttl: Duration::from_secs(DEFAULT_TTL_SECS),
        }
    }

    /// Create a simulator with custom TTL.
    #[cfg(test)]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            last_fingerprint: None,
            last_timestamp: None,
            ttl,
        }
    }

    /// Compute a deterministic fingerprint from the request prefix.
    ///
    /// The prefix is: system prompt + tools + all messages except the last
    /// (the user's current input). This matches how real providers determine
    /// cacheable prefixes.
    pub fn compute_fingerprint(messages: &[MessageEntry], tools: &[String]) -> u64 {
        let mut hasher = FnvHasher::new();

        // Hash system prompt (first message if role="system").
        let has_system = messages.first().is_some_and(|m| m.role == "system");
        if has_system {
            hasher.write_bytes(messages[0].content.as_bytes());
        }

        // Hash tool definitions (sorted for determinism).
        let mut sorted_tools = tools.to_vec();
        sorted_tools.sort();
        for tool in &sorted_tools {
            hasher.write_bytes(tool.as_bytes());
        }

        // Hash non-system messages in the prefix (all except the last).
        // Skip system prompt at index 0 to avoid double-hashing.
        let prefix_end = messages.len().saturating_sub(1);
        let start = if has_system { 1 } else { 0 };
        for msg in &messages[start..prefix_end] {
            hasher.write_bytes(msg.role.as_bytes());
            hasher.write_bytes(msg.content.as_bytes());
        }

        // Return 0 for completely empty prefix (no system prompt, no tools,
        // no prior messages) to ensure determinism.
        if prefix_end <= start && sorted_tools.is_empty() && !has_system {
            return 0;
        }

        hasher.finish()
    }

    /// Estimate token count from prefix content (deterministic, approximate).
    ///
    /// Returns the approximate token count of the cacheable prefix: system
    /// prompt + tools + all messages except the last. Returns 0 when there
    /// is no cacheable prefix.
    fn estimate_prefix_tokens(messages: &[MessageEntry], tools: &[String]) -> u32 {
        let mut char_count = 0usize;

        if let Some(first) = messages.first() {
            if first.role == "system" {
                char_count += first.content.len();
            }
        }

        for tool in tools {
            char_count += tool.len();
        }

        let prefix_len = messages.len().saturating_sub(1);
        for msg in &messages[..prefix_len] {
            char_count += msg.content.len();
        }

        if char_count == 0 {
            return 0;
        }

        // ~4 chars per token (approximate, deterministic).
        // Minimum 1 token to ensure non-zero for non-empty prefixes.
        ((char_count / 4) as u32).max(1)
    }

    /// Process a request and return cache simulation result.
    ///
    /// `explicit_hit` / `explicit_write`: scenario-declared override values.
    /// When `Some`, these take priority over auto simulation.
    pub fn process(
        &mut self,
        messages: &[MessageEntry],
        tools: &[String],
        explicit_hit: Option<u32>,
        explicit_write: Option<u32>,
    ) -> CacheResult {
        let fingerprint = Self::compute_fingerprint(messages, tools);

        // Explicit injection override — state machine records but does not
        // influence the returned values.
        if explicit_hit.is_some() || explicit_write.is_some() {
            let state = match &self.last_fingerprint {
                None => CacheState::Writing,
                Some(fp) => {
                    if *fp == fingerprint {
                        CacheState::Hit
                    } else {
                        CacheState::Writing
                    }
                }
            };
            // Record state for next request.
            self.last_fingerprint = Some(fingerprint);
            self.last_timestamp = Some(Instant::now());
            return CacheResult {
                cache_hit_tokens: explicit_hit,
                cache_write_tokens: explicit_write,
                state,
            };
        }

        // Auto simulation — state machine logic.
        let prefix_tokens = Self::estimate_prefix_tokens(messages, tools);

        let (hit_tokens, write_tokens, new_state) = match &self.last_fingerprint {
            None => {
                // State: Empty → Writing
                (0, prefix_tokens, CacheState::Writing)
            }
            Some(last_fp) => {
                if *last_fp == fingerprint {
                    // Same prefix — check TTL.
                    let expired = self
                        .last_timestamp
                        .map(|ts| ts.elapsed() > self.ttl)
                        .unwrap_or(false);

                    if expired {
                        // State: Hit → Expired → Writing (rewrite)
                        (0, prefix_tokens, CacheState::Writing)
                    } else {
                        // State: Hit → Hit (cache hit)
                        (prefix_tokens, 0, CacheState::Hit)
                    }
                } else {
                    // Prefix changed — State: Break → Writing
                    (0, prefix_tokens, CacheState::Writing)
                }
            }
        };

        // Record state for next request.
        self.last_fingerprint = Some(fingerprint);
        self.last_timestamp = Some(Instant::now());

        CacheResult {
            cache_hit_tokens: if hit_tokens > 0 {
                Some(hit_tokens)
            } else {
                None
            },
            cache_write_tokens: if write_tokens > 0 {
                Some(write_tokens)
            } else {
                None
            },
            state: new_state,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> MessageEntry {
        MessageEntry {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn first_request_writes_cache() {
        let mut sim = KvCacheSimulator::new();
        let msgs = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
        ];
        let result = sim.process(&msgs, &[], None, None);

        assert_eq!(result.state, CacheState::Writing);
        assert!(result.cache_hit_tokens.is_none());
        assert!(result.cache_write_tokens.is_some());
        assert!(result.cache_write_tokens.unwrap() > 0);
    }

    #[test]
    fn same_prefix_hits_cache() {
        let mut sim = KvCacheSimulator::new();
        // First request: system + user + assistant + user. Prefix = system + user + assistant.
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "next"),
        ];
        let r1 = sim.process(&msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        // Second request: same prefix, different last message.
        // Prefix = system + user + assistant (same as msgs1's full list).
        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process(&msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);
        assert!(r2.cache_hit_tokens.is_some());
        assert!(r2.cache_hit_tokens.unwrap() > 0);
        assert!(r2.cache_write_tokens.is_none());
    }

    #[test]
    fn prefix_change_breaks_cache() {
        let mut sim = KvCacheSimulator::new();
        // First request: system + user + assistant + user.
        // Prefix = system + user + assistant.
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        let r1 = sim.process(&msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        // Second request, same prefix: hit.
        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process(&msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);

        // Third request, different system prompt: break → write.
        let msgs3 = vec![
            msg("system", "You are a different assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        let r3 = sim.process(&msgs3, &[], None, None);
        assert_eq!(r3.state, CacheState::Writing);
        assert!(r3.cache_write_tokens.is_some());
    }

    #[test]
    fn ttl_expiry_forces_rewrite() {
        let mut sim = KvCacheSimulator::with_ttl(Duration::from_millis(10));
        let msgs = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
        ];

        // First request: write.
        let r1 = sim.process(&msgs, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        // Wait for TTL to expire.
        std::thread::sleep(Duration::from_millis(20));

        // Second request after expiry: expired → write.
        let r2 = sim.process(&msgs, &[], None, None);
        assert_eq!(r2.state, CacheState::Writing);
        assert!(r2.cache_write_tokens.is_some());
    }

    #[test]
    fn explicit_injection_overrides_auto() {
        let mut sim = KvCacheSimulator::new();
        let msgs = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
        ];

        // First request with explicit values.
        let r1 = sim.process(&msgs, &[], Some(100), Some(200));
        assert_eq!(r1.cache_hit_tokens, Some(100));
        assert_eq!(r1.cache_write_tokens, Some(200));

        // Second request: auto simulation runs, explicit values applied.
        let r2 = sim.process(&msgs, &[], Some(50), None);
        assert_eq!(r2.cache_hit_tokens, Some(50));
        assert!(r2.cache_write_tokens.is_none());
    }

    #[test]
    fn explicit_injection_with_none_falls_back_to_auto() {
        let mut sim = KvCacheSimulator::new();
        // First request: auto simulation (write).
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "next"),
        ];
        let r1 = sim.process(&msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        // Second request: same prefix, explicit_hit overrides auto hit.
        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process(&msgs2, &[], Some(999), None);
        assert_eq!(r2.cache_hit_tokens, Some(999));
        assert_eq!(r2.state, CacheState::Hit);
    }

    #[test]
    fn fingerprint_deterministic() {
        let msgs = vec![msg("user", "hello")];
        let fp1 = KvCacheSimulator::compute_fingerprint(&msgs, &[]);
        let fp2 = KvCacheSimulator::compute_fingerprint(&msgs, &[]);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_differs_on_content_change() {
        // Content change in the prefix (not the last message).
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi there"),
            msg("user", "follow up"),
        ];
        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "different topic"),
            msg("assistant", "hi there"),
            msg("user", "follow up"),
        ];
        let fp1 = KvCacheSimulator::compute_fingerprint(&msgs1, &[]);
        let fp2 = KvCacheSimulator::compute_fingerprint(&msgs2, &[]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_differs_on_tool_change() {
        let msgs = vec![msg("user", "hello")];
        let fp1 = KvCacheSimulator::compute_fingerprint(&msgs, &[]);
        let fp2 = KvCacheSimulator::compute_fingerprint(&msgs, &["search".to_string()]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_stable_prefix_only() {
        // Prefix = system + user("hello") + assistant("hi").
        // Both have the same prefix (all except last message).
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "next"),
        ];
        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let fp1 = KvCacheSimulator::compute_fingerprint(&msgs1, &[]);
        let fp2 = KvCacheSimulator::compute_fingerprint(&msgs2, &[]);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn estimate_prefix_tokens_positive() {
        let msgs = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello world"),
        ];
        let tokens = KvCacheSimulator::estimate_prefix_tokens(&msgs, &[]);
        assert!(tokens > 0);
    }

    #[test]
    fn estimate_prefix_tokens_includes_tools() {
        let msgs = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hi"),
        ];
        let t1 = KvCacheSimulator::estimate_prefix_tokens(&msgs, &[]);
        let t2 = KvCacheSimulator::estimate_prefix_tokens(&msgs, &["search".to_string()]);
        assert!(t2 > t1);
    }
}
