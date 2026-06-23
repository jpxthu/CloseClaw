//! Renderer — rendering layer infrastructure.
//!
//! Platform-specific rendering logic (Feishu, Terminal) now lives in the
//! corresponding `IMPlugin` implementations under `src/im/`.

pub mod streaming {
    pub use crate::im_adapter::streaming::*;
}

pub use crate::im_adapter::RenderedOutput;
