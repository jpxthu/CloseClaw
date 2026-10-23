//! Crate-wide shared constants for the CLI.

/// Default agent used when `chat --agent-id` is omitted: the initial `master`
/// agent created by the config wizard (docs/design/agent/agent-config.md).
pub(crate) const DEFAULT_AGENT_ID: &str = "master";
