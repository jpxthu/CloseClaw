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
//!
//! # Memory config three-layer merge
//!
//! Memory config follows a three-layer merge: global `memory.json`
//! (base) → user-level agent memory (middle) → project-level agent
//! memory (top). Each layer's declared fields override the layer below;
//! undeclared fields inherit from below.

use std::path::PathBuf;

use crate::agents::config_types::{AgentConfig, MemoryConfig, ModelSpec, SubagentsConfig};
use crate::ConfigError;
use closeclaw_common::BootstrapMode;

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

/// Return project's Vec if non-empty, otherwise fall back to user's.
///
/// Implements the design doc rule: "project non-empty value replaces
/// user value" for Vec fields (skills, tools, disallowed_tools, allow_agents).
/// Wildcard `["*"]` is treated as a normal non-empty value.
fn override_if_non_empty<T>(project: Vec<T>, user: Vec<T>) -> Vec<T> {
    if !project.is_empty() {
        project
    } else {
        user
    }
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
    pub model: Option<ModelSpec>,
    pub workspace: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub bootstrap_mode: BootstrapMode,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub subagents: SubagentsConfig,
    pub memory: crate::agents::config_types::MemoryConfig,
    /// Which configuration level this was resolved from.
    pub source: ConfigSource,
}

impl ResolvedAgentConfig {
    /// Check whether a list is a wildcard (empty or `["*"]`), meaning
    /// "no filtering — allow all".
    pub fn is_wildcard_list(list: &[String]) -> bool {
        list.is_empty() || list == ["*"]
    }
    /// Return the effective skills whitelist.
    ///
    /// Returns `None` when the list is wildcard (empty or `["*"]`), meaning
    /// no filtering applies. Otherwise returns `Some(whitelist)`.
    pub fn effective_skills(&self) -> Option<Vec<String>> {
        if Self::is_wildcard_list(&self.skills) {
            None
        } else {
            Some(self.skills.clone())
        }
    }
    /// Return the effective tools whitelist.
    ///
    /// Returns `None` when the list is wildcard (empty or `["*"]`), meaning
    /// no filtering applies. Otherwise returns `Some(whitelist)`.
    pub fn effective_tools(&self) -> Option<Vec<String>> {
        if Self::is_wildcard_list(&self.tools) {
            None
        } else {
            Some(self.tools.clone())
        }
    }
    /// Return the effective disallowed tools blacklist.
    ///
    /// Returns `None` when the list is empty (no tools are disallowed).
    /// A non-empty list means those tools are explicitly blocked.
    pub fn effective_disallowed_tools(&self) -> Option<Vec<String>> {
        if self.disallowed_tools.is_empty() {
            None
        } else {
            Some(self.disallowed_tools.clone())
        }
    }
}

impl ResolvedAgentConfig {
    /// Convert a single `AgentConfig` into a resolved form, tagging it
    /// with the given `source` level. The `path` argument is used purely
    /// for error reporting when `id` validation fails.
    ///
    /// If `global_memory` is provided, the agent's memory config is merged
    /// on top of the global defaults (agent fields override global fields).
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
        global_memory: Option<&MemoryConfig>,
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
        let memory = match (global_memory, config.memory) {
            (Some(global), Some(agent)) => global.merge_overrides(&agent),
            (Some(global), None) => global.clone(),
            (None, Some(agent)) => agent,
            (None, None) => MemoryConfig::default(),
        };
        Ok(Self {
            id: config.id,
            name,
            parent_id: config.parent_id,
            model: config.model,
            workspace: config.workspace.map(PathBuf::from),
            agent_dir: config.agent_dir.map(PathBuf::from),
            bootstrap_mode: config.bootstrap_mode.unwrap_or(BootstrapMode::Full),
            skills: config.skills,
            tools: config.tools,
            disallowed_tools: config.disallowed_tools,
            subagents: apply_subagent_defaults(config.subagents),
            memory,
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
    /// Memory config follows a three-layer merge: `global_memory` (base)
    /// → user-level agent memory (middle) → project-level agent memory (top).
    ///
    /// Name resolution: project's non-empty value wins, otherwise the
    /// user's non-empty value, otherwise the resolved `id` is used as
    /// a fallback. `None` and `Some("")` are both treated as "not
    /// provided" at each level.
    ///
    /// Returns [`ConfigError::MissingId`] when the resolved `id` is
    /// empty after the project-then-user fallback.
    pub fn merge(
        project: AgentConfig,
        user: AgentConfig,
        path: &str,
        global_memory: Option<&MemoryConfig>,
    ) -> Result<Self, ConfigError> {
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
            bootstrap_mode: project
                .bootstrap_mode
                .or(user.bootstrap_mode)
                .unwrap_or(BootstrapMode::Full),
            skills: override_if_non_empty(project.skills, user.skills),
            tools: override_if_non_empty(project.tools, user.tools),
            disallowed_tools: override_if_non_empty(
                project.disallowed_tools,
                user.disallowed_tools,
            ),
            subagents: merge_subagents(project.subagents, user.subagents),
            memory: {
                // Three-layer merge: global (base) → user (middle) → project (top)
                let base = global_memory.cloned().unwrap_or_default();
                let after_user = match &user.memory {
                    Some(u) => base.merge_overrides(u),
                    None => base,
                };
                match &project.memory {
                    Some(p) => after_user.merge_overrides(p),
                    None => after_user,
                }
            },
            source: ConfigSource::Merged,
        })
    }
}

impl TryFrom<AgentConfig> for ResolvedAgentConfig {
    type Error = ConfigError;

    /// Convert via [`ResolvedAgentConfig::from_single`], defaulting the
    /// source to [`ConfigSource::User`] and passing no global memory config.
    /// Callers that know the source or have global config should call
    /// [`ResolvedAgentConfig::from_single`] directly. The `path` used for
    /// error reporting is `"<unknown>"` since the `TryFrom` trait does not
    /// expose a source location.
    fn try_from(config: AgentConfig) -> Result<Self, Self::Error> {
        Self::from_single(config, ConfigSource::User, "<unknown>", None)
    }
}

/// Default subagent values used when neither project nor user config
/// specifies a value (both `None`).
const DEFAULT_MAX_SPAWN_DEPTH: u32 = 1;
const DEFAULT_MAX_CHILDREN: u32 = 5;
const DEFAULT_REQUIRE_AGENT_ID: bool = false;

/// Apply defaults to a single [`SubagentsConfig`], filling `None` fields
/// with their canonical values. Used by both `from_single` and `merge`
/// paths to keep defaulting consistent.
fn apply_subagent_defaults(mut config: SubagentsConfig) -> SubagentsConfig {
    if config.require_agent_id.is_none() {
        config.require_agent_id = Some(DEFAULT_REQUIRE_AGENT_ID);
    }
    if config.max_spawn_depth.is_none() {
        config.max_spawn_depth = Some(DEFAULT_MAX_SPAWN_DEPTH);
    }
    if config.max_children.is_none() {
        config.max_children = Some(DEFAULT_MAX_CHILDREN);
    }
    config
}

/// Field-level merge for [`SubagentsConfig`]; mirrors the rules used in
/// [`ResolvedAgentConfig::merge`].
///
/// For `Option<T>` fields: project's `Some` wins, otherwise user's `Some`,
/// otherwise the field's default value.
fn merge_subagents(project: SubagentsConfig, user: SubagentsConfig) -> SubagentsConfig {
    let merged = SubagentsConfig {
        allow_agents: override_if_non_empty(project.allow_agents, user.allow_agents),
        require_agent_id: project.require_agent_id.or(user.require_agent_id),
        max_spawn_depth: project.max_spawn_depth.or(user.max_spawn_depth),
        max_children: project.max_children.or(user.max_children),
        timeout: project.timeout.or(user.timeout),
        default_child_agent: project.default_child_agent.or(user.default_child_agent),
        model: project.model.or(user.model),
    };
    apply_subagent_defaults(merged)
}
