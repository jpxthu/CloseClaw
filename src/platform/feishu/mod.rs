//! Feishu Platform Adapter Module
//!
//! Provides the [`FeishuAdapter`] which handles Feishu-specific behaviour,
//! most notably the Stream → Plan fallback when stream output is not fully
//! supported by the platform.

pub mod adapter;
pub mod adapter_event;
pub mod card;
pub mod card_updater;
pub mod complexity;
pub mod error;
pub mod fallback;
pub mod updater;

pub use adapter::{FallbackResult, FeishuAdapter};
pub use card_updater::CardService;
