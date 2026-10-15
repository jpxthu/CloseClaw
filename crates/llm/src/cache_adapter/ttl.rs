//! Provider prompt-cache TTL knowledge.
//!
//! Split out of `mod.rs` to keep the module root free of substantive
//! definitions (CONTRIBUTING: mod.rs should only hold `pub use` /
//! `pub mod`).

/// Default provider prompt-cache validity period, in milliseconds.
///
/// The generic 5-minute magnitude used for providers without an
/// explicitly recorded TTL (see [`provider_cache_ttl_ms`]).
pub const DEFAULT_PROVIDER_CACHE_TTL_MS: u64 = 300_000;

/// Returns the provider's prompt-cache validity period in milliseconds.
///
/// TTL knowledge for the cache natural-expiry exemption of the cache
/// hit consistency check (session does not depend on `closeclaw-llm`,
/// so the gateway resolves this and passes it in as a parameter).
///
/// Magnitude sources: Anthropic's prompt cache is valid for 5 minutes
/// (docs/design/llm/cache-adapter.md "Anthropic 适配"); MiniMax follows
/// the same 5-minute magnitude (docs/design/llm/providers/minimax.md
/// "缓存机制"). Providers without a recorded TTL use the generic
/// default of the same magnitude (`300_000`).
pub fn provider_cache_ttl_ms(provider_id: &str) -> u64 {
    match provider_id {
        "anthropic" | "minimax" => 300_000,
        _ => DEFAULT_PROVIDER_CACHE_TTL_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::{provider_cache_ttl_ms, DEFAULT_PROVIDER_CACHE_TTL_MS};

    // ------------------------------------------------------------------
    // provider_cache_ttl_ms — cache natural-expiry TTL knowledge
    // ------------------------------------------------------------------

    /// Providers with an explicitly recorded TTL (docs/design/llm/
    /// cache-adapter.md "Anthropic 适配" + docs/design/llm/providers/
    /// minimax.md "缓存机制") report 5 minutes.
    #[test]
    fn provider_cache_ttl_ms_recorded_providers() {
        assert_eq!(provider_cache_ttl_ms("anthropic"), 300_000);
        assert_eq!(provider_cache_ttl_ms("minimax"), 300_000);
    }

    /// Unrecorded providers use the generic 5-minute default
    /// (docs/design/session/llm-session-enhancements.md "不参与校验的
    /// 情形": 未单独收录有效期的供应商按通用默认有效期处理).
    #[test]
    fn provider_cache_ttl_ms_unrecorded_providers_default() {
        for provider_id in [
            "openai",
            "deepseek",
            "mimo",
            "kimi",
            "glm",
            "volcengine",
            "",
        ] {
            assert_eq!(
                provider_cache_ttl_ms(provider_id),
                DEFAULT_PROVIDER_CACHE_TTL_MS,
                "expected default TTL for provider_id: {provider_id:?}"
            );
        }
    }

    /// The default constant and the recorded values share the same
    /// 5-minute magnitude — recorded entries exist to document the
    /// source, not to deviate from the default.
    #[test]
    fn provider_cache_ttl_ms_default_matches_recorded_magnitude() {
        assert_eq!(DEFAULT_PROVIDER_CACHE_TTL_MS, 300_000);
        assert_eq!(
            provider_cache_ttl_ms("anthropic"),
            DEFAULT_PROVIDER_CACHE_TTL_MS
        );
    }
}
