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
pub mod sections;
pub mod tools_section;
pub mod workdir;

pub use builder::{
    build_from_workspace, build_system_prompt, set_agent_prompt, set_custom_prompt,
    set_override_prompt, WorkspaceBuildConfig,
};
pub use sections::{
    clear_append_section, get_append_section, get_cached_section, invalidate_all_sections,
    invalidate_section, set_append_section, Section,
};
pub use tools_section::build_tools_section;
pub use workdir::{build_git_status_for, build_workdir_context, WorkdirContext};

/// Maximum character length for append_section content
pub const APPEND_SECTION_MAX_LEN: usize = sections::APPEND_SECTION_MAX_LEN;
