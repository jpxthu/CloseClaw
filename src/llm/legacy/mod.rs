//! LLM legacy module — bridges old and new provider architectures.
//!
//! [`LegacyProviderBridge`] implements the new [`Provider`] trait
//! by wrapping a concrete type that implements the old [`LLMProvider`] trait.
//! This allows incremental migration from `LLMProvider` to `Provider`
//! without breaking existing provider implementations.

pub mod legacy_provider;
pub mod legacy_session;
