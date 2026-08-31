//! Integration test binary: cross-module tests that do not spin up a full stack.
//!
//! Each test case lives in its own module; `main.rs` only declares modules.
//! See docs/developer/STANDARDS.md for the e2e/integration/unit classification.

mod compaction_async_tests;
mod debug_log_integration_tests;
mod fake_integration_tests;
mod feishu_message_cleaner_tests;
mod gateway_send_outbound_basic_tests;
mod gateway_send_outbound_renderer_tests;
mod im_inbound_tests;
mod integration_llm_busy_tests;
mod integration_memory_pipeline_tests;
mod integration_pending_messages_tests;
mod integration_permission_tests;
mod integration_tests;
mod minimax_mock_tests;
mod minimax_stream_mock_tests;
mod plan_mode_integration_tests;
mod rejection_log_archive_integration_tests;
