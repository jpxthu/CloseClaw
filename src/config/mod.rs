//! Configuration System - hot-reloadable JSON configs
//!
//! Implements ConfigProvider trait for extensible config management.

pub mod providers;
pub use providers::ConfigProvider;
