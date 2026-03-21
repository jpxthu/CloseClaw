//! Configuration System - hot-reloadable JSON configs
//!
//! Implements ConfigProvider trait for extensible config management.

pub mod providers;
pub mod agents;
pub mod backup;
pub mod reload;
pub use providers::{ConfigProvider, ConfigError};
