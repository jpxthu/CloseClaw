//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! The shared resolved-config type family migrated to `closeclaw_common`
//! (issue #3344); this crate keeps the raw `AgentConfig` and construction
//! logic.
//! Design: `docs/design/agent/agent-config.md`

use serde::{Deserialize, Serialize};

use closeclaw_common::{BootstrapMode, HookConfig, MemoryConfig, ModelSpec, SubagentsConfig};

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
    /// Whether the agent explicitly declares no bootstrap files
    /// (`noBootstrap` in config.json). When `true`, the agent's sessions
    /// run as "sessions without bootstrap files".
    #[serde(default)]
    pub no_bootstrap: Option<bool>,
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
            no_bootstrap: None,
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
