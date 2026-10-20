//! Integration test binary for the tools crate: cross-module tests that
//! do not spin up a full stack.
//!
//! Each test case lives in its own module; `main.rs` only declares
//! modules. See docs/developer/STANDARDS.md for the
//! e2e/integration/unit classification.

mod plan_write_concurrency_tests;
