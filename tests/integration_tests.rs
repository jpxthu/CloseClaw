//! Integration Tests for CloseClaw
//!
//! Tests in this file verify cross-module interactions.
//! All unit tests have been moved to their respective modules:
//!   - Permission engine tests → `src/permission/engine.rs`
//!   - Validation tests → `src/permission/rules/mod.rs`
//!   - Skill tests → `src/skills/builtin.rs`
//!   - Agent tests → `src/agent/mod.rs`
//!   - Config tests → `src/config/mod.rs`

// Note: All previously inlined permission engine, validation, skill registry,
// agent creation, and permission engine parse tests have been moved to their
// respective `src/` module `mod tests` blocks.
