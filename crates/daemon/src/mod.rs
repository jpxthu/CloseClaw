//! Daemon - CloseClaw background service
//!
//! Orchestrates all components: Gateway, AgentRegistry, PermissionEngine.
//! Handles graceful shutdown via ShutdownCoordinator.
mod agent_permissions_adapter;
pub mod bridge;
pub mod chat_rpc;
pub mod config_helpers;
pub mod config_reload;
pub mod config_watcher;
pub mod confirm_notify;
mod daemon_struct;
pub mod dreaming_scheduler;
mod env_file;
pub mod gateway_restart;
pub mod lifecycle;
mod memory_params_adapter;
mod memory_storage_adapter;
mod metrics;
pub mod phase_init;
pub mod phase_wiring;
pub mod plan_file_store_adapter;
pub mod processor_registry;
pub mod read_truncation_adapter;
pub mod registries;
pub mod shutdown;
pub(crate) mod shutdown_heartbeat;
pub mod skill_access_adapter;
pub mod skill_reload;
pub mod startup;
pub mod tool_permission_adapter;
pub mod tool_skill_access_adapter;
pub mod trait_adapters;
pub mod workflow_launcher_adapter;
pub mod workflow_port_adapter;
#[cfg(test)]
pub(crate) use closeclaw_config::{ConfigManager, ConfigSection};
#[cfg(test)]
pub(crate) use closeclaw_gateway::GatewayConfig;
pub use closeclaw_gateway::SpawnController;
#[cfg(test)]
pub(crate) use closeclaw_memory::dreaming::DreamingPipeline;
pub use daemon_struct::*;
pub(crate) use env_file::load_env_file;
#[cfg(test)]
pub(crate) use env_file::parse_env_file;
#[cfg(test)]
pub(crate) use phase_init::ServiceShutdownReceivers;
#[cfg(test)]
pub(crate) use std::sync::Arc;
#[cfg(test)]
mod dreaming_scheduler_tests;
#[cfg(test)]
mod gateway_restart_checkpoint_tests;
#[cfg(test)]
mod lifecycle_abort_tests;
#[cfg(test)]
mod lifecycle_assembly_tests;
#[cfg(test)]
mod lifecycle_phase3_heartbeat_tests;
#[cfg(test)]
mod lifecycle_tests;
mod llm_components;
mod llm_init;
mod noop_miner_llm;
#[cfg(test)]
mod processor_registry_tests;
#[cfg(test)]
mod session_config_provider_tests;
#[cfg(test)]
mod shutdown_alignment_tests;
#[cfg(test)]
mod shutdown_tests;
mod skills_helper;
#[cfg(test)]
#[path = "spawn_controller_crate_reexport_tests.rs"]
mod spawn_controller_crate_reexport_tests;
mod startup_plan;
#[cfg(test)]
mod step14_comprehensive_tests;
#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
mod test_helpers_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unit_tests;
