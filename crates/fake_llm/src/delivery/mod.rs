//! Delivery layer for the Fake LLM Server.
//!
//! Responsible for SSE event generation (OpenAI and Anthropic protocols),
//! delay injection, error injection (with Retry-After header), and the
//! unified delivery entry point that routes requests through the
//! appropriate response path.
//!
//! See `docs/design/fake_llm/delivery.md` for the full specification.

pub mod inject;
pub mod sse;

// Re-export public types from sub-modules.
pub use inject::{apply_delay, deliver, DeliveryConfig, DeliveryResult, Protocol, StreamInterrupt};
pub use sse::{
    generate_anthropic_sse, generate_openai_sse, split_segments, to_axum_event, SseEvent,
    SseEventStream, DEFAULT_SEGMENT_GRANULARITY,
};

#[cfg(test)]
mod tests;
