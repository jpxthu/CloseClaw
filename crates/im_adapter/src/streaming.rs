//! StreamingRenderer — re-exported from `closeclaw_common::streaming`.
//!
//! The actual implementation lives in `closeclaw-common`. This module
//! re-exports types for backward compatibility with existing imports.

pub use closeclaw_common::im_plugin::StreamingOutput;
pub use closeclaw_common::streaming::{DefaultStreamingRenderer, LineBuffer, StreamingRenderer};
