//! IM Adapters - Protocol implementations for various messaging platforms
//!
//! Each adapter implements the IMAdapter trait for a specific IM platform.
//! Feishu plugin lives under `im_adapter::platforms::feishu`.

pub mod processor;
pub mod terminal;
#[cfg(test)]
pub mod terminal_tests;

// Re-export from im_adapter for backward compatibility
pub use crate::im_adapter::{AdapterError, IMAdapter, IMPlugin, NormalizedMessage};

// Re-export feishu types for backward compatibility
pub use crate::im_adapter::platforms::feishu::{
    build_text, CachedToken, FeishuAdapter, FeishuPlugin,
};
