//! E2E test binary: drives real process composition stacks.
//!
//! Each test case lives in its own module; `main.rs` only declares modules.
//! See docs/testing/STANDARDS.md for the e2e/integration/unit classification.

mod integration_shutdown_checkpoint_tests;
mod sandbox_integration_tests;
mod sigterm_integration_tests;
