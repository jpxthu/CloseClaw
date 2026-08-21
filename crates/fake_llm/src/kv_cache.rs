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
    /// Whether this result represents a cache break (prefix changed).
    pub is_break: bool,
}

/// KV cache prefix simulation state machine.
///
/// Tracks the last prefix fingerprint and timestamp, computes deterministic
/// cache hit/write token counts based on state transitions.
pub struct KvCacheSimulator {
    last_fingerprint: Option<u64>,
    last_timestamp: Option<Instant>,
    ttl: Duration,
    /// Previous request's prefix messages (for common-prefix computation).
    old_prefix_messages: Vec<MessageEntry>,
    /// Previous request's prefix tools (for common-prefix computation).
    old_prefix_tools: Vec<String>,
}

impl Default for KvCacheSimulator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl KvCacheSimulator {
    /// Create a simulator with default TTL (5 minutes).
    pub fn new() -> Self {
        Self {
            last_fingerprint: None,
            last_timestamp: None,
            ttl: Duration::from_secs(DEFAULT_TTL_SECS),
            old_prefix_messages: Vec::new(),
            old_prefix_tools: Vec::new(),
        }
    }

    /// Create a simulator with custom TTL.
    #[cfg(test)]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            last_fingerprint: None,
            last_timestamp: None,
            ttl,
            old_prefix_messages: Vec::new(),
            old_prefix_tools: Vec::new(),
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
        let prefix_end = messages.len().saturating_sub(1);
        let start = if has_system { 1 } else { 0 };
        for msg in &messages[start..prefix_end] {
            hasher.write_bytes(msg.role.as_bytes());
            hasher.write_bytes(msg.content.as_bytes());
        }

        // Return 0 for completely empty prefix to ensure determinism.
        if prefix_end <= start && sorted_tools.is_empty() && !has_system {
            return 0;
        }

        hasher.finish()
    }
}

// ---------------------------------------------------------------------------
// Utility — token estimation & common-prefix helpers
// ---------------------------------------------------------------------------

impl KvCacheSimulator {
    /// Compute the number of tokens in the common prefix of two sets of
    /// prefix components (system prompt + tools + messages).
    ///
    /// Compares element-by-element: system prompt first, then sorted tools,
    /// then prefix messages. Stops at the first mismatch and returns the
    /// cumulative token count of the matching portion.
    fn common_prefix_tokens(
        old_msgs: &[MessageEntry],
        old_tools: &[String],
        new_msgs: &[MessageEntry],
        new_tools: &[String],
    ) -> u32 {
        let mut char_count = 0usize;

        // Compare system prompts.
        let old_has_sys = old_msgs.first().is_some_and(|m| m.role == "system");
        let new_has_sys = new_msgs.first().is_some_and(|m| m.role == "system");
        if old_has_sys && new_has_sys {
            if old_msgs[0].content != new_msgs[0].content {
                return 0;
            }
            char_count += old_msgs[0].content.len();
        } else if old_has_sys != new_has_sys {
            return 0;
        }

        // Compare sorted tools.
        let mut old_sorted = old_tools.to_vec();
        old_sorted.sort();
        let mut new_sorted = new_tools.to_vec();
        new_sorted.sort();
        let tool_count = old_sorted.len().min(new_sorted.len());
        for i in 0..tool_count {
            if old_sorted[i] != new_sorted[i] {
                return Self::tokens_from_chars(char_count);
            }
            char_count += old_sorted[i].len();
        }
        if old_sorted.len() != new_sorted.len() {
            return Self::tokens_from_chars(char_count);
        }

        // Compare prefix messages (all except the last message in each).
        let old_sys_offset = if old_has_sys { 1 } else { 0 };
        let new_sys_offset = if new_has_sys { 1 } else { 0 };
        let old_prefix_end = old_msgs.len().saturating_sub(1);
        let new_prefix_end = new_msgs.len().saturating_sub(1);
        let old_non_sys = &old_msgs[old_sys_offset..old_prefix_end];
        let new_non_sys = &new_msgs[new_sys_offset..new_prefix_end];
        let msg_count = old_non_sys.len().min(new_non_sys.len());
        for i in 0..msg_count {
            if old_non_sys[i].role != new_non_sys[i].role
                || old_non_sys[i].content != new_non_sys[i].content
            {
                return Self::tokens_from_chars(char_count);
            }
            char_count += old_non_sys[i].content.len();
        }

        Self::tokens_from_chars(char_count)
    }

    /// Convert character count to approximate token count.
    fn tokens_from_chars(char_count: usize) -> u32 {
        if char_count == 0 {
            0
        } else {
            ((char_count / 4) as u32).max(1)
        }
    }

    /// Estimate token count from prefix content (deterministic,
    /// approximate). Returns the approximate token count of the cacheable
    /// prefix: system prompt + tools + all messages except the last.
    /// Returns 0 when there is no cacheable prefix.
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

        Self::tokens_from_chars(char_count)
    }
}

// ---------------------------------------------------------------------------
// Simulation — state machine core
// ---------------------------------------------------------------------------

impl KvCacheSimulator {
    /// Run auto-simulation state machine and return computed fields:
    /// (hit_tokens, write_tokens, new_state, is_break, was_expired).
    fn auto_simulate(
        &self,
        messages: &[MessageEntry],
        tools: &[String],
        fingerprint: u64,
    ) -> (u32, u32, CacheState, bool, bool) {
        let prefix_tokens = Self::estimate_prefix_tokens(messages, tools);
        match &self.last_fingerprint {
            None => {
                // State: Empty → Writing
                (0, prefix_tokens, CacheState::Writing, false, false)
            }
            Some(last_fp) => {
                if *last_fp == fingerprint {
                    // Same prefix — check TTL.
                    let expired = self
                        .last_timestamp
                        .map(|ts| ts.elapsed() > self.ttl)
                        .unwrap_or(false);
                    if expired {
                        // Hit → Expired → Writing (rewrite)
                        (0, prefix_tokens, CacheState::Writing, false, true)
                    } else {
                        // Hit → Hit (cache hit)
                        (prefix_tokens, 0, CacheState::Hit, false, false)
                    }
                } else {
                    // Prefix changed — Break: compute residual hit from
                    // common prefix, remaining as write tokens.
                    let residual = Self::common_prefix_tokens(
                        &self.old_prefix_messages,
                        &self.old_prefix_tools,
                        messages,
                        tools,
                    );
                    let write = prefix_tokens.saturating_sub(residual);
                    (residual, write, CacheState::Writing, true, false)
                }
            }
        }
    }
    /// Emit tracing log for cache events (auto-simulation path only).
    fn log_cache_event(
        hit_tokens: u32,
        write_tokens: u32,
        state: &CacheState,
        is_break: bool,
        was_expired: bool,
    ) {
        match state {
            CacheState::Hit => {
                tracing::info!(
                    target: "fake_llm::kv_cache",
                    hit_tokens,
                    "cache hit"
                );
            }
            CacheState::Writing if is_break => {
                tracing::info!(
                    target: "fake_llm::kv_cache",
                    residual_hit = hit_tokens,
                    write_tokens,
                    "cache break"
                );
            }
            CacheState::Writing if was_expired => {
                tracing::info!(
                    target: "fake_llm::kv_cache",
                    "cache expired"
                );
            }
            CacheState::Writing => {
                tracing::info!(
                    target: "fake_llm::kv_cache",
                    write_tokens,
                    "cache write"
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Process — request orchestration
// ---------------------------------------------------------------------------

impl KvCacheSimulator {
    /// Process a request and return cache simulation result.
    ///
    /// `scenario_name`: identifies which scenario this request belongs to.
    /// Used for per-scenario state isolation in the engine layer.
    ///
    /// `explicit_hit` / `explicit_write`: scenario-declared override values.
    /// When `Some`, these take priority over auto simulation.
    pub fn process(
        &mut self,
        _scenario_name: &str,
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
                Some(fp) if *fp == fingerprint => CacheState::Hit,
                _ => CacheState::Writing,
            };
            // Record state for next request (including old prefix data
            // so subsequent auto-simulation can compute residual hits).
            self.last_fingerprint = Some(fingerprint);
            self.last_timestamp = Some(Instant::now());
            self.old_prefix_messages = messages.to_vec();
            self.old_prefix_tools = tools.to_vec();
            return CacheResult {
                cache_hit_tokens: explicit_hit,
                cache_write_tokens: explicit_write,
                state,
                is_break: false,
            };
        }

        // Auto simulation — state machine logic.
        let (hit, write, state, brk, exp) = self.auto_simulate(messages, tools, fingerprint);

        // Record state for next request.
        self.last_fingerprint = Some(fingerprint);
        self.last_timestamp = Some(Instant::now());
        self.old_prefix_messages = messages.to_vec();
        self.old_prefix_tools = tools.to_vec();

        // Observability logging (auto-simulation path only).
        Self::log_cache_event(hit, write, &state, brk, exp);

        CacheResult {
            cache_hit_tokens: if hit > 0 { Some(hit) } else { None },
            cache_write_tokens: if write > 0 { Some(write) } else { None },
            state,
            is_break: brk,
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
        let result = sim.process("test", &msgs, &[], None, None);

        assert_eq!(result.state, CacheState::Writing);
        assert!(result.cache_hit_tokens.is_none());
        assert!(result.cache_write_tokens.is_some());
        assert!(result.cache_write_tokens.unwrap() > 0);
    }

    #[test]
    fn same_prefix_hits_cache() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "next"),
        ];
        let r1 = sim.process("test", &msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);
        assert!(r2.cache_hit_tokens.is_some());
        assert!(r2.cache_hit_tokens.unwrap() > 0);
        assert!(r2.cache_write_tokens.is_none());
    }

    #[test]
    fn prefix_change_breaks_cache() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        let r1 = sim.process("test", &msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);

        let msgs3 = vec![
            msg("system", "You are a different assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, None);
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

        let r1 = sim.process("test", &msgs, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        std::thread::sleep(Duration::from_millis(20));

        let r2 = sim.process("test", &msgs, &[], None, None);
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

        let r1 = sim.process("test", &msgs, &[], Some(100), Some(200));
        assert_eq!(r1.cache_hit_tokens, Some(100));
        assert_eq!(r1.cache_write_tokens, Some(200));

        let r2 = sim.process("test", &msgs, &[], Some(50), None);
        assert_eq!(r2.cache_hit_tokens, Some(50));
        assert!(r2.cache_write_tokens.is_none());
    }

    #[test]
    fn explicit_injection_with_none_falls_back_to_auto() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "next"),
        ];
        let r1 = sim.process("test", &msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process("test", &msgs2, &[], Some(999), None);
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

    // ------------------------------------------------------------------
    // Step 1.3: Residual hit tests
    // ------------------------------------------------------------------

    #[test]
    fn break_residual_same_system_prompt() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        let r1 = sim.process("test", &msgs1, &[], None, None);
        assert_eq!(r1.state, CacheState::Writing);

        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);

        // Different message content, same system prompt.
        let msgs3 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "new topic"),
            msg("assistant", "hi"),
            msg("user", "another"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, None);
        assert_eq!(r3.state, CacheState::Writing);
        assert!(r3.is_break);
        assert!(r3.cache_hit_tokens.is_some());
        let residual = r3.cache_hit_tokens.unwrap();
        assert!(residual > 0);
        assert!(r3.cache_write_tokens.is_some());
        assert!(r3.cache_write_tokens.unwrap() > 0);
    }

    #[test]
    fn break_residual_completely_different() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "follow up"),
        ];
        sim.process("test", &msgs1, &[], None, None);

        let msgs2 = vec![
            msg("system", "You are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "different"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);

        let msgs3 = vec![
            msg("system", "New system prompt entirely"),
            msg("user", "completely new"),
            msg("assistant", "ok"),
            msg("user", "another"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, None);
        assert_eq!(r3.state, CacheState::Writing);
        assert!(r3.is_break);
        assert!(r3.cache_hit_tokens.is_none());
        assert!(r3.cache_write_tokens.is_some());
        assert!(r3.cache_write_tokens.unwrap() > 0);
    }

    #[test]
    fn break_residual_partial_message_overlap() {
        let mut sim = KvCacheSimulator::new();
        let msgs1 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "old question"),
        ];
        sim.process("test", &msgs1, &[], None, None);

        let msgs2 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "another"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert_eq!(r2.state, CacheState::Hit);

        let msgs3 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "different response"),
            msg("user", "new"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, None);
        assert_eq!(r3.state, CacheState::Writing);
        assert!(r3.is_break);
        assert!(r3.cache_hit_tokens.is_some());
        let residual = r3.cache_hit_tokens.unwrap();
        assert!(residual > 0);
        let total = KvCacheSimulator::estimate_prefix_tokens(&msgs3, &[]);
        assert!(r3.cache_write_tokens.is_some());
        assert!(r3.cache_write_tokens.unwrap() < total);
    }

    #[test]
    fn is_break_flag_true_only_on_break() {
        let mut sim = KvCacheSimulator::new();

        let msgs1 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q1"),
        ];
        let r1 = sim.process("test", &msgs1, &[], None, None);
        assert!(!r1.is_break);

        let msgs2 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q2"),
        ];
        let r2 = sim.process("test", &msgs2, &[], None, None);
        assert!(!r2.is_break);

        let msgs3 = vec![
            msg("system", "new sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q3"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, None);
        assert!(r3.is_break);

        let msgs4 = vec![
            msg("system", "new sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q4"),
        ];
        let r4 = sim.process("test", &msgs4, &[], None, None);
        assert!(!r4.is_break);
    }

    #[test]
    fn explicit_injection_correct_values_no_logging() {
        let mut sim = KvCacheSimulator::new();
        let msgs = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q1"),
        ];

        let r1 = sim.process("test", &msgs, &[], Some(100), Some(200));
        assert_eq!(r1.cache_hit_tokens, Some(100));
        assert_eq!(r1.cache_write_tokens, Some(200));
        assert!(!r1.is_break);

        let msgs2 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q2"),
        ];
        let r2 = sim.process("test", &msgs2, &[], Some(500), None);
        assert_eq!(r2.cache_hit_tokens, Some(500));
        assert!(r2.cache_write_tokens.is_none());
        assert!(!r2.is_break);
        assert_eq!(r2.state, CacheState::Hit);

        let msgs3 = vec![
            msg("system", "different sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q3"),
        ];
        let r3 = sim.process("test", &msgs3, &[], None, Some(300));
        assert!(r3.cache_hit_tokens.is_none());
        assert_eq!(r3.cache_write_tokens, Some(300));
        assert!(!r3.is_break);
    }

    // ------------------------------------------------------------------
    // Step 1.4: Regression test for explicit injection → auto-sim break
    // ------------------------------------------------------------------

    #[test]
    fn explicit_then_auto_break_residual_correct() {
        // Verify that after explicit injection, switching back to
        // auto-simulation correctly computes break residual hit using
        // the old prefix recorded during explicit injection.
        let mut sim = KvCacheSimulator::new();

        // Request 1: explicit injection (records prefix A).
        let msgs_a = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q1"),
        ];
        let r1 = sim.process("test", &msgs_a, &[], Some(100), Some(200));
        assert_eq!(r1.cache_hit_tokens, Some(100));

        // Request 2: explicit injection with same prefix (records again).
        let msgs_a2 = vec![
            msg("system", "sys"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "q2"),
        ];
        let r2 = sim.process("test", &msgs_a2, &[], Some(50), None);
        assert_eq!(r2.cache_hit_tokens, Some(50));

        // Request 3: auto-simulation with different prefix → break.
        // old_prefix was set during explicit injection, so residual
        // hit should be computed from prefix A's data.
        let msgs_b = vec![
            msg("system", "new sys"),
            msg("user", "world"),
            msg("assistant", "yo"),
            msg("user", "q3"),
        ];
        let r3 = sim.process("test", &msgs_b, &[], None, None);
        assert_eq!(r3.state, CacheState::Writing);
        assert!(r3.is_break);
        // Completely different prefix → no residual hit.
        assert!(r3.cache_hit_tokens.is_none());
        assert!(r3.cache_write_tokens.is_some());
        assert!(r3.cache_write_tokens.unwrap() > 0);

        // Request 4: auto-simulation, same prefix as r3 → hit.
        let msgs_b2 = vec![
            msg("system", "new sys"),
            msg("user", "world"),
            msg("assistant", "yo"),
            msg("user", "q4"),
        ];
        let r4 = sim.process("test", &msgs_b2, &[], None, None);
        assert_eq!(r4.state, CacheState::Hit);
        assert!(!r4.is_break);

        // Request 5: auto-simulation with partially overlapping prefix
        // (same system prompt as msgs_b) → break with residual.
        let msgs_c = vec![
            msg("system", "new sys"),
            msg("user", "different"),
            msg("assistant", "ok"),
            msg("user", "q5"),
        ];
        let r5 = sim.process("test", &msgs_c, &[], None, None);
        assert_eq!(r5.state, CacheState::Writing);
        assert!(r5.is_break);
        // System prompt "new sys" is common → residual > 0.
        assert!(r5.cache_hit_tokens.is_some());
        assert!(r5.cache_hit_tokens.unwrap() > 0);
    }
}
