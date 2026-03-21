//! CloseClaw — Lightweight, rule-driven multi-agent execution framework
//!
//! Core components:
//! - **Permission Engine**: Independent process for rule evaluation
//! - **Agent Runtime**: Manages agent lifecycle and inter-agent communication
//! - **Gateway**: IM protocol adapters (Feishu, Wecom, QQ, DingTalk, etc.)
//! - **Config System**: Hot-reloadable JSON configs with validation and rollback

pub mod permission;
pub mod agent;
pub mod gateway;
pub mod config;
pub mod im;
pub mod skills;
pub mod llm;
pub mod cli;
pub mod daemon;

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
