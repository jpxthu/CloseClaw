//! Resolved agent configuration and sub-agent spawn control types.
//!
//! [`ResolvedAgentConfig`] is the fully-merged, defaults-filled view of a
//! per-agent `config.json`: the config crate loads both levels (user /
//! project), merges them, and fills defaults, producing this type for
//! downstream consumers (Session, Tool Registry, Skill Registry,
//! Permission, System Prompt, Gateway, Daemon). The construction and
//! merge logic itself lives in the config crate.
//!
//! Design: `docs/design/common/shared-types.md` §ResolvedAgentConfig /
//! SubagentsConfig / MemoryConfig, `docs/design/agent/agent-config.md`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{BootstrapMode, HookConfig, MemoryConfig, ModelSpec};

/// Sub-agent spawn control configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsConfig {
    /// Whitelist of allowed target agent IDs; `["*"]` means no restriction.
    #[serde(default = "default_all")]
    pub allow_agents: Vec<String>,
    /// Whether agentId must be explicitly specified when spawning.
    #[serde(default)]
    pub require_agent_id: Option<bool>,
    /// Maximum nested spawn depth.
    #[serde(default)]
    pub max_spawn_depth: Option<u32>,
    /// Maximum concurrent active child sessions.
    #[serde(default)]
    pub max_children: Option<u32>,
    /// Sub-agent maximum execution duration (seconds).
    /// Falls back to global config when unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Sub-agent expected execution duration (seconds).
    /// When the sub-agent has been running for this many seconds,
    /// cyclic warning notifications begin. Falls back to global
    /// default (legacy single warning) when unspecified.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "timeoutWarning"
    )]
    pub timeout_warning: Option<u64>,
    /// Interval ratio for cyclic warning notifications (relative to timeout_warning).
    /// Must be >=0.1 and <=2.0, default 0.5 (50% of timeout_warning).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "timeoutNotifyIntervalRatio"
    )]
    pub timeout_notify_interval_ratio: Option<f64>,
    /// Default child agent ID (used when spawn omits agentId).
    ///
    /// **Deprecated**: This field is no longer used in agentId resolution.
    /// When spawn omits agentId, the parent agent's own ID is always used
    /// (design doc §Spawn 控制流程 ④). Kept for config file compatibility;
    /// ignored by SpawnController.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[deprecated(note = "Ignored: spawn now always uses parent agent ID as default")]
    pub default_child_agent: Option<String>,
    /// Model override for child agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
}

fn default_all() -> Vec<String> {
    vec!["*".to_string()]
}

impl Default for SubagentsConfig {
    #[allow(deprecated)] // default_child_agent is deprecated; included for config backward compat
    fn default() -> Self {
        Self {
            allow_agents: default_all(),
            require_agent_id: None,
            max_spawn_depth: None,
            max_children: None,
            timeout: None,
            timeout_warning: None,
            timeout_notify_interval_ratio: None,
            default_child_agent: None,
            model: None,
        }
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
    /// Whether the agent config explicitly declares no bootstrap files
    /// (config.json `noBootstrap`; defaults to `false`, meaning bootstrap
    /// files load normally per `bootstrap_mode`).
    pub no_bootstrap: bool,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub subagents: SubagentsConfig,
    pub memory: MemoryConfig,
    /// Whether the agent config explicitly set a `memory` field.
    /// When `false`, the `memory` value is inherited from global defaults
    /// and `agent info` should report `null` per the design doc.
    pub memory_configured: bool,
    /// Run-health hook review configuration.
    pub hooks: Vec<HookConfig>,
    /// Whether parallel tool calls are enabled for this agent.
    /// When `false`, all tool calls are executed serially.
    pub parallel_tool_calls: bool,
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
