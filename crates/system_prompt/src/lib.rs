//! System Prompt Architecture Module
//!
//! Provides a layered System Prompt building system with:
//! - Static section caching (role, workspace, tools, memory, heartbeat)
//! - Dynamic section per-request injection (channel_context, session_state, append)
//! - Workdir context and gitStatus integration
//! - `/system`, `/cd`, `/pwd`, `/git` slash commands
//!
//! Issue: #166

pub mod builder;
pub mod fragment;
pub mod inject;
pub mod providers;
pub mod sections;
pub mod tools_section;
pub mod workdir;

pub use builder::{
    build_from_workspace, build_system_prompt, PromptOverrides, WorkspaceBuildConfig,
};
pub use closeclaw_common;
pub use fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};
pub use providers::bootstrap::BootstrapFragmentProvider;
pub use providers::memory::MemoryFragmentProvider;
pub use providers::skills::SkillsFragmentProvider;
pub use providers::tools::ToolsFragmentProvider;
pub use sections::{get_cached_section, invalidate_all_sections, invalidate_section, Section};
pub use tools_section::build_tools_section;
pub use workdir::{build_git_status_for, build_workdir_context, WorkdirContext};
