//! Resolved agent configuration after two-level merge.
//!
//! Combines user-level (`~/.closeclaw/agents/<id>/`) and project-level
//! (`<repo>/.closeclaw/agents/<id>/`) agent configurations into a single
//! [`ResolvedAgentConfig`] that downstream modules (Session, Tool Registry,
//! Skill Registry, etc.) can consume without further fallback logic.
//!
//! # Merge rules
//!
//! - `Option` fields: project's `Some` wins, otherwise fall back to the
//!   user value, otherwise fall back to the field's own default.
//! - `Vec` fields (`skills`, `tools`, `disallowed_tools`, `allow_agents`):
//!   project's non-empty value replaces the user's; otherwise the user
//!   value is kept; otherwise the field's default applies.
//! - `bootstrap_mode`, scalar `SubagentsConfig` fields: a non-default
//!   project value overrides the user; otherwise the user value is kept.
//! - Required `String` fields (`id`, `name`): project wins when non-empty,
//!   otherwise fall back to the user value.

use std::path::PathBuf;

use crate::agent::config::{AgentConfig, SubagentsConfig};
use crate::session::bootstrap::BootstrapMode;

/// Configuration source level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// Loaded from user-level config only.
    User,
    /// Loaded from project-level config only.
    Project,
    /// Merged from both levels (project fields override user fields).
    Merged,
}

/// Fully resolved agent configuration after two-level merge.
///
/// All optional fields have been filled with defaults where neither
/// project nor user config specified a value.
#[derive(Debug, Clone)]
pub struct ResolvedAgentConfig {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub model: Option<String>,
    pub workspace: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub bootstrap_mode: BootstrapMode,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub subagents: SubagentsConfig,
    /// Which configuration level this was resolved from.
    pub source: ConfigSource,
}

impl ResolvedAgentConfig {
    /// Convert a single `AgentConfig` into a resolved form, tagging it
    /// with the given `source` level. No fallback is performed; fields
    /// come straight from `config`.
    pub fn from_single(config: AgentConfig, source: ConfigSource) -> Self {
        Self {
            id: config.id,
            name: config.name,
            parent_id: config.parent_id,
            model: config.model,
            workspace: config.workspace.map(PathBuf::from),
            agent_dir: config.agent_dir.map(PathBuf::from),
            bootstrap_mode: config.bootstrap_mode,
            skills: config.skills,
            tools: config.tools,
            disallowed_tools: config.disallowed_tools,
            subagents: config.subagents,
            source,
        }
    }

    /// Merge project-level and user-level configs into a resolved form.
    ///
    /// Project fields take precedence over user fields; see the module
    /// documentation for the full rule set. The resulting `source` is
    /// always [`ConfigSource::Merged`].
    pub fn merge(project: AgentConfig, user: AgentConfig) -> Self {
        let id = if !project.id.is_empty() {
            project.id
        } else {
            user.id
        };
        let name = if !project.name.is_empty() {
            project.name
        } else {
            user.name
        };

        Self {
            id,
            name,
            parent_id: project.parent_id.or(user.parent_id),
            model: project.model.or(user.model),
            workspace: project
                .workspace
                .map(PathBuf::from)
                .or_else(|| user.workspace.map(PathBuf::from)),
            agent_dir: project
                .agent_dir
                .map(PathBuf::from)
                .or_else(|| user.agent_dir.map(PathBuf::from)),
            bootstrap_mode: if project.bootstrap_mode != BootstrapMode::Full {
                project.bootstrap_mode
            } else {
                user.bootstrap_mode
            },
            skills: if !project.skills.is_empty() && project.skills != ["*"] {
                project.skills
            } else {
                user.skills
            },
            tools: if !project.tools.is_empty() && project.tools != ["*"] {
                project.tools
            } else {
                user.tools
            },
            disallowed_tools: if !project.disallowed_tools.is_empty() {
                project.disallowed_tools
            } else {
                user.disallowed_tools
            },
            subagents: merge_subagents(project.subagents, user.subagents),
            source: ConfigSource::Merged,
        }
    }
}

impl From<AgentConfig> for ResolvedAgentConfig {
    /// Convert via [`ResolvedAgentConfig::from_single`], defaulting the
    /// source to [`ConfigSource::User`]. Callers that know the source
    /// should call [`ResolvedAgentConfig::from_single`] directly.
    fn from(config: AgentConfig) -> Self {
        Self::from_single(config, ConfigSource::User)
    }
}

// Defaults that mirror `SubagentsConfig::default()` in
// `crate::agent::config`. Used to detect whether a project-level value
// explicitly overrode the user-level value during field-level merging.
const SUBAGENT_DEFAULT_MAX_SPAWN_DEPTH: u32 = 1;
const SUBAGENT_DEFAULT_MAX_CHILDREN: u32 = 5;

/// Field-level merge for [`SubagentsConfig`]; mirrors the rules used in
/// [`ResolvedAgentConfig::merge`].
fn merge_subagents(project: SubagentsConfig, user: SubagentsConfig) -> SubagentsConfig {
    SubagentsConfig {
        allow_agents: if !project.allow_agents.is_empty() && project.allow_agents != ["*"] {
            project.allow_agents
        } else {
            user.allow_agents
        },
        require_agent_id: project.require_agent_id || user.require_agent_id,
        max_spawn_depth: if project.max_spawn_depth != SUBAGENT_DEFAULT_MAX_SPAWN_DEPTH {
            project.max_spawn_depth
        } else {
            user.max_spawn_depth
        },
        max_children: if project.max_children != SUBAGENT_DEFAULT_MAX_CHILDREN {
            project.max_children
        } else {
            user.max_children
        },
        default_child_agent: project.default_child_agent.or(user.default_child_agent),
        model: project.model.or(user.model),
    }
}
