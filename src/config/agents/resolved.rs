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
//! - `id` (required, validated at the end): project's non-empty value
//!   wins, otherwise the user's non-empty value. A fully empty `id`
//!   after merging is rejected with [`ConfigError::MissingId`].
//! - `name` (optional, falls back to `id`): project's non-empty value
//!   wins, otherwise the user's non-empty value, otherwise falls back
//!   to the resolved `id`. `None` and `Some("")` are both treated as
//!   "not provided" and trigger the fallback.

use std::path::PathBuf;

use crate::agent::config::{AgentConfig, AgentPermissions, SubagentsConfig};
use crate::config::ConfigError;
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
    /// Inline permissions for this agent.
    /// When present, takes priority over external permissions.json.
    pub permissions: Option<AgentPermissions>,
    /// Which configuration level this was resolved from.
    pub source: ConfigSource,
}

impl ResolvedAgentConfig {
    /// Convert a single `AgentConfig` into a resolved form, tagging it
    /// with the given `source` level. The `path` argument is used purely
    /// for error reporting when `id` validation fails.
    ///
    /// Name fallback: a missing (`None`) or empty (`Some("")`) `name`
    /// falls back to `id`. Both levels are treated as "not provided"
    /// to keep the behavior consistent with the design doc
    /// ("name 默认同 id").
    ///
    /// Returns [`ConfigError::MissingId`] when the resolved `id` is
    /// empty after the fallback chain.
    pub fn from_single(
        config: AgentConfig,
        source: ConfigSource,
        path: &str,
    ) -> Result<Self, ConfigError> {
        if config.id.is_empty() {
            return Err(ConfigError::MissingId {
                path: path.to_string(),
            });
        }
        let name = config
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| config.id.clone());

        Ok(Self {
            id: config.id,
            name,
            parent_id: config.parent_id,
            model: config.model,
            workspace: config.workspace.map(PathBuf::from),
            agent_dir: config.agent_dir.map(PathBuf::from),
            bootstrap_mode: config.bootstrap_mode,
            skills: config.skills,
            tools: config.tools,
            disallowed_tools: config.disallowed_tools,
            subagents: config.subagents,
            permissions: config.permissions,
            source,
        })
    }

    /// Merge project-level and user-level configs into a resolved form.
    ///
    /// Project fields take precedence over user fields; see the module
    /// documentation for the full rule set. The resulting `source` is
    /// always [`ConfigSource::Merged`]. The `path` argument is used
    /// purely for error reporting when `id` validation fails.
    ///
    /// Name resolution: project's non-empty value wins, otherwise the
    /// user's non-empty value, otherwise the resolved `id` is used as
    /// a fallback. `None` and `Some("")` are both treated as "not
    /// provided" at each level.
    ///
    /// Returns [`ConfigError::MissingId`] when the resolved `id` is
    /// empty after the project-then-user fallback.
    pub fn merge(project: AgentConfig, user: AgentConfig, path: &str) -> Result<Self, ConfigError> {
        let id = if !project.id.is_empty() {
            project.id
        } else {
            user.id
        };
        if id.is_empty() {
            return Err(ConfigError::MissingId {
                path: path.to_string(),
            });
        }
        let name = project
            .name
            .filter(|n| !n.is_empty())
            .or_else(|| user.name.filter(|n| !n.is_empty()))
            .unwrap_or_else(|| id.clone());

        Ok(Self {
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
            permissions: project.permissions.or(user.permissions),
            source: ConfigSource::Merged,
        })
    }
}

impl TryFrom<AgentConfig> for ResolvedAgentConfig {
    type Error = ConfigError;

    /// Convert via [`ResolvedAgentConfig::from_single`], defaulting the
    /// source to [`ConfigSource::User`]. Callers that know the source
    /// should call [`ResolvedAgentConfig::from_single`] directly. The
    /// `path` used for error reporting is `"<unknown>"` since the
    /// `TryFrom` trait does not expose a source location.
    fn try_from(config: AgentConfig) -> Result<Self, Self::Error> {
        Self::from_single(config, ConfigSource::User, "<unknown>")
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
