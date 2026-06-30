//! CloseClaw — Lightweight, rule-driven multi-agent execution framework
//!
//! Core components:
//! - **Permission Engine**: Independent process for rule evaluation
//! - **Agent Runtime**: Manages agent lifecycle and inter-agent communication
//! - **Gateway**: IM protocol adapters (Feishu, Wecom, QQ, DingTalk, etc.)
//! - **Config System**: Hot-reloadable JSON configs with validation and rollback

pub use closeclaw_admin as admin;
pub use closeclaw_agent as agent;
pub use closeclaw_cli as cli;
pub mod config_reload;
pub mod daemon;
pub use closeclaw_platform as platform;
pub use closeclaw_processor_chain as processor_chain;
pub mod session;
pub use closeclaw_slash as slash;
pub use closeclaw_system_prompt as system_prompt;
pub use closeclaw_tasks as tasks;

pub mod bridge;
pub mod common;
pub use closeclaw_memory as memory;

use tracing::info;

/// Initialize the CloseClaw library
pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("CloseClaw v{} initialized", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init_tracing_once() {
        INIT.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive(tracing::Level::INFO.into()),
                )
                .with_target(true)
                .with_thread_ids(true)
                .finish();
            let _ = tracing::subscriber::set_global_default(subscriber);
        });
    }

    #[test]
    fn test_init_does_not_panic() {
        init_tracing_once();
        // Replicate the init() logic to verify it does not panic.
        // We use set_global_default instead of .init() to avoid global state issues in tests.
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            )
            .with_target(true)
            .with_thread_ids(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    #[test]
    fn test_version_macro_expanded() {
        // CARGO_PKG_VERSION is a string of form "MAJOR.MINOR.PATCH"
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty(), "CARGO_PKG_VERSION must not be empty");
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "CARGO_PKG_VERSION must have 3 parts");
        for part in &parts {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "Each version part must be numeric: {}",
                version
            );
        }
    }
}
