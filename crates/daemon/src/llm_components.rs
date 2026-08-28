//! Per-provider LLM component assembly (protocol, interpreter, plugin).
//!
//! Re-exports the canonical implementation from `closeclaw_llm::call_chain`
//! to avoid duplication. Used by `lifecycle_assembly_tests`.
//!
//! See also: `docs/design/llm/README.md` § 五层架构

#[allow(unused_imports)]
pub(crate) use closeclaw_llm::call_chain::assemble_llm_components;
