//! E2E test binary: drives real process composition stacks.
//!
//! Each test case lives in its own module; `main.rs` only declares modules.
//! See docs/developer/STANDARDS.md for the e2e/integration/unit classification.

mod agent_profile_tests;
mod sandbox_tests;
mod shutdown_checkpoint_tests;
mod sigterm_tests;
