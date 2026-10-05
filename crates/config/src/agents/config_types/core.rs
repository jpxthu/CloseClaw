//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! Migrated from `closeclaw-common::agent_config`.
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

use serde::{Deserialize, Serialize};

use super::MemoryConfig;
use closeclaw_common::{BootstrapMode, HookConfig, ModelSpec};

/// Agent's own configuration (stored as config.json in the agent's directory).
///
/// Permissions are stored in a separate `permissions.json` file, not inline
/// in `config.json` (per design doc).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Parent agent ID (if this agent was spawned by another).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Default LLM model for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
    /// Working directory path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Directory for bootstrap files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_dir: Option<String>,
    /// Bootstrap file loading mode.
    #[serde(default)]
    pub bootstrap_mode: Option<BootstrapMode>,
    /// Available skill names; `["*"]` means all skills are available.
    #[serde(default = "default_all")]
    pub skills: Vec<String>,
    /// Available tool names whitelist.
    #[serde(default = "default_all")]
    pub tools: Vec<String>,
    /// Disallowed tool names blacklist.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Sub-agent spawn control parameters.
    #[serde(default)]
    pub subagents: SubagentsConfig,
    /// Memory subsystem configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
    /// Run-health hook review configuration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookConfig>,
    /// Whether parallel tool calls are enabled for this agent.
    /// When `false`, all tool calls are executed serially.
    #[serde(default = "default_true")]
    pub parallel_tool_calls: bool,
}

fn default_all() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_true() -> bool {
    true
}

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

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: None,
            skills: default_all(),
            tools: default_all(),
            disallowed_tools: Vec::new(),
            subagents: SubagentsConfig::default(),
            memory: None,
            hooks: Vec::new(),
            parallel_tool_calls: true,
        }
    }
}
