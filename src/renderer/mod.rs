//! Renderer — rendering layer infrastructure.
//!
//! This module provides the core types and trait for rendering LLM output
//! to platform-specific formats:
//! - [`RenderedOutput`] — output type from a renderer
//! - [`Renderer`] — trait for platform-specific renderers
//!
//! # Example
//! ```
//! use closeclaw::renderer::{Renderer, RenderedOutput};
//! use closeclaw::renderer::feishu::FeishuRenderer;
//! use closeclaw::llm::types::ContentBlock;
//!
//! let renderer = FeishuRenderer::new();
//! // A single Text block containing inline formatting (**) → interactive card.
//! // With dsl_result=None there are no DSL buttons in the payload.
//! let blocks = vec![ContentBlock::Text("Hello **world**".to_string())];
//! let output = renderer.render(&blocks, None);
//! assert_eq!(output.msg_type, "interactive");
//! // No DSL → no action/button elements in the card
//! let elements = output.payload.get("card").and_then(|c| c.get("elements")).and_then(|e| e.as_array());
//! assert!(elements.is_some());
//! assert!(elements.unwrap().iter().all(|e| e.get("tag").and_then(|t| t.as_str()) != Some("action")));
//! ```

pub mod feishu;
pub use crate::im_adapter::{code_block, streaming};

pub use crate::im_adapter::{RenderedOutput, Renderer};
