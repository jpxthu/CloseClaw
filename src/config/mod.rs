//! Configuration System - hot-reloadable JSON configs
//!
//! Implements ConfigProvider trait for extensible config management.

pub mod agents;
pub mod backup;
pub mod providers;
pub mod reload;
pub use agents::{AgentDirectoryEntry, AgentDirectoryProvider, AgentsConfig, AgentsConfigProvider};
pub use providers::{ConfigError, ConfigProvider};

#[cfg(test)]
mod tests {
    use super::*;

    // From tests/smoke_test.rs
    #[test]
    fn test_config_provider_trait_exists() {
        fn _check<T: ConfigProvider>() {}
    }
}
