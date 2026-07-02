//! Standard [`PromptFragmentProvider`] implementations.
//!
//! Each provider contributes one section of the system prompt static layer.
//! Providers are sorted by [`PromptFragmentProvider::priority`] (lower first)
//! and their non-empty outputs are concatenated by the Builder.

pub mod bootstrap;
