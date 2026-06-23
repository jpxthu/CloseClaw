//! Renderer — rendering layer infrastructure.
//!
//! This module provides the core types and trait for rendering LLM output
//! to platform-specific formats:
//! - [`RenderedOutput`] — output type from a renderer
//!
//! # Example
//! ```
//! use closeclaw::renderer::RenderedOutput;
//!
//! let output = RenderedOutput {
//!     msg_type: "text".into(),
//!     payload: serde_json::json!("Hello"),
//! };
//! assert_eq!(output.msg_type, "text");
//! ```

pub mod feishu;
pub mod terminal;
#[cfg(test)]
pub mod terminal_tests;
pub use crate::im_adapter::{code_block, streaming};

pub use crate::im_adapter::RenderedOutput;
