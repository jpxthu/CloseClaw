//! Session glue layer in root crate.
//!
//! Contains `execute_compact` — the LLM-calling bridge that depends on
//! `closeclaw-llm` (which would create a cycle if placed in `crates/session`).

pub mod compaction;
