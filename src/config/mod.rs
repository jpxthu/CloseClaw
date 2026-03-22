//! Configuration System - hot-reloadable JSON configs
//!
//! Implements ConfigProvider trait for extensible config management.

pub mod providers;
pub mod agents;
pub mod backup;
pub mod reload;
pub use providers::{ConfigProvider, ConfigError};

#[cfg(test)]
mod tests {
    use super::*;

    // From tests/smoke_test.rs
    #[test]
    fn test_config_provider_trait_exists() {
        fn _check<T: ConfigProvider>() {}
    }
}
