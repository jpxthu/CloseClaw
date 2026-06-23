//! Renderer — rendering layer infrastructure.
//!
//! This module re-exports core rendering types for convenience:
//! - [`RenderedOutput`] — output type from a renderer
//!
//! Platform-specific rendering logic (Feishu, Terminal) now lives in the
//! corresponding `IMPlugin` implementations under `src/im/`.

pub use crate::im_adapter::{code_block, streaming};

pub use crate::im_adapter::RenderedOutput;
