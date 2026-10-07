//! System Prompt Architecture Module
//!
//! Provides a layered System Prompt building system with:
//! - Static section caching (role, workspace, tools, memory, heartbeat)
//! - Dynamic section per-request injection
//!   (channel_context, working_directory, mode_instruction, git_status)
//! - Workdir context and gitStatus integration
//! - `/system`, `/cd`, `/pwd`, `/git` slash commands
//!
//! Issue: #166

pub mod adapter;
pub mod builder;
mod fragment;
pub mod inject;
pub mod providers;
pub mod sections;
mod workdir;

#[cfg(test)]
pub mod test_adapters;

#[cfg(test)]
pub mod inject_tests;

#[cfg(test)]
pub mod inject_appends_tests;

pub use builder::{build_from_workspace, build_system_prompt, WorkspaceBuildConfig};
pub use inject::{DynamicSectionsParams, SystemPromptDynamicBuilder};
pub use providers::bootstrap::{BootstrapFragmentProvider, BootstrapListFn, BootstrapLoadFn};
pub use sections::{Section, SectionCache};
