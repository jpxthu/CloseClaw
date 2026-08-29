//! Standard [`PromptFragmentProvider`] implementations.
//!
//! Each provider contributes one section of the system prompt static layer.
//! Providers are sorted by [`PromptFragmentProvider::priority`] (lower first)
//! and their non-empty outputs are concatenated by the Builder.
//!
//! The three domain providers (Tools, Skills, Memory) have been migrated to
//! their respective crates. Only BootstrapFragmentProvider remains here as
//! it is the system_prompt crate's own provider.

pub mod bootstrap;
