//! E2E/008 — Thinking tag persistence and compact boundary tests
//!
//! Verifies that `<thinking>` tags survive `chat_history` storage,
//! are stripped from Compact boundary messages, and correctly
//! traverse the TCP wire protocol.
//!
//! Run with: `cargo test --features fake-llm --test e2e_thinking -- --test-threads=1`

#![allow(deprecated)]

#[path = "e2e_thinking/mod.rs"]
mod e2e_thinking;
