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
pub mod slash_commands;
pub mod workdir;

pub use builder::{
    build_from_workspace, build_system_prompt, set_agent_prompt, set_custom_prompt,
    set_override_prompt,
};
pub use sections::{
    clear_append_section, get_append_section, get_cached_section, invalidate_all_sections,
    invalidate_section, set_append_section, Section,
};
pub use workdir::{get_workdir, set_workdir, WorkdirContext};

/// Maximum character length for append_section content
pub const APPEND_SECTION_MAX_LEN: usize = sections::APPEND_SECTION_MAX_LEN;
