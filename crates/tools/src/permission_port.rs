//! Permission check port for the tools crate.
//!
//! [`ToolPermissionCheck`] is the tools-owned abstraction over the
//! permission engine, approval flow, session lookups, and config lookups,
//! narrowed to the exact call surface of the two-level permission checks
//! ([`crate::permission_check`]) and [`crate::builtin::BashTool`]'s
//! trust-level routing: evaluate a tool-permission request (with the
//! session/no-session branch resolved inside the implementation), submit
//! a denial to the approval flow, decide sub-agent sessions, and
//! classify config-file paths.
//!
//! The mirror types below carry only the variants and fields tools
//! actually constructs or consumes — `InterAgentMsg` / `SlashCommand`
//! requests and the `Allowed` token / `context_modifier` / `Denied.rule`
//! response fields have zero consumers in this crate and are therefore
//! not mirrored.
//!
//! The composition root (daemon) supplies the production implementation
//! wrapping `closeclaw_permission`'s engine + approval flow; tests may
//! use the adapter in `test_adapters` (permission is a dev-dependency of
//! this crate).

use async_trait::async_trait;

/// Caller metadata for a permission request or denial submission.
///
/// Mirror of the permission-side `Caller`; `user_id` is empty for
/// "system caller" requests (e.g. Bash trust-level routing, which runs
/// before user identity resolution).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermCaller {
    /// The user ID of the message source (empty = unresolved).
    pub user_id: String,
    /// The agent instance ID (always present).
    pub agent: String,
}

/// Risk level assigned to a denied operation (tools-owned mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermRiskLevel {
    /// Low risk.
    Low,
    /// Medium risk.
    Medium,
    /// High risk.
    High,
    /// Critical risk (e.g. malicious command detection).
    Critical,
}

/// Direction of message flow for the MessageSend dimension
/// (tools-owned mirror).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PermMessageDirection {
    /// Outbound messages.
    Send,
    /// Inbound messages.
    Receive,
    /// Both directions.
    #[default]
    Both,
}

/// The permission request body (tools-owned mirror).
///
/// Only the seven construction sites in this crate are mirrored;
/// `InterAgentMsg` / `SlashCommand` are not constructed here.
#[derive(Debug, Clone)]
pub enum PermRequestBody {
    /// Agent is invoking a tool (skill group + method).
    ToolCall {
        /// Requesting agent ID.
        agent: String,
        /// Tool group being invoked.
        skill: String,
        /// Method on the tool.
        method: String,
    },
    /// File read/write access.
    FileOp {
        /// Requesting agent ID.
        agent: String,
        /// Target path.
        path: String,
        /// `"read"` or `"write"`.
        op: String,
    },
    /// Message send/receive.
    MessageSend {
        /// Requesting agent ID.
        agent: String,
        /// Direction of the message flow.
        direction: PermMessageDirection,
        /// Target conversation/channel.
        target: String,
    },
    /// Write access to a config file.
    ConfigWrite {
        /// Requesting agent ID.
        agent: String,
        /// Config file path.
        config_file: String,
    },
    /// Network access to a host/port.
    NetOp {
        /// Requesting agent ID.
        agent: String,
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
    },
    /// Shell command execution.
    CommandExec {
        /// Requesting agent ID.
        agent: String,
        /// Base command.
        cmd: String,
        /// Command arguments.
        args: Vec<String>,
    },
}

/// Verdict of a permission evaluation (tools-owned mirror).
///
/// `Allowed` carries no fields — the engine's token and
/// `context_modifier` have zero consumers in this crate. `Denied` drops
/// the `rule` field, which is likewise unread here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermVerdict {
    /// Operation is permitted.
    Allowed,
    /// Operation is denied.
    Denied {
        /// Human-readable denial reason.
        reason: String,
        /// Assessed risk level of the denied operation.
        risk_level: PermRiskLevel,
        /// When set, the operation was already submitted to the approval
        /// flow and is pending owner approval; callers must not
        /// re-submit.
        approval_request_id: Option<String>,
    },
}

/// Tools-owned port over permission evaluation and denial routing.
///
/// Implementations must be `Send + Sync` (tools are shared behind
/// `Arc`).
#[async_trait]
pub trait ToolPermissionCheck: Send + Sync {
    /// Evaluate a permission request.
    ///
    /// When `session_id` is `Some`, the implementation resolves the
    /// session sender, upgrades the request to caller-carrying form, and
    /// evaluates with parent-agent chain intersection; otherwise it
    /// evaluates the bare request. When a sender is resolved, its user
    /// ID is written back into `caller` so the denial path can reuse the
    /// resolved identity without a second lookup.
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        caller: &mut PermCaller,
        body: &PermRequestBody,
    ) -> PermVerdict;

    /// Submit a denied operation to the approval flow.
    ///
    /// Returns `Some(request_id)` when the flow accepted the request
    /// (owner notified / queued), `None` when it rejected it
    /// (sub-agent / duplicate).
    async fn submit_denial(
        &self,
        caller: &PermCaller,
        body: &PermRequestBody,
        risk_level: PermRiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String>;

    /// Determine whether the given session belongs to a sub-agent
    /// (depth > 0). Returns `false` for empty or unresolvable session
    /// ids — the safe default.
    async fn is_session_sub_agent(&self, session_id: &str) -> bool;

    /// Whether `path` lies inside the managed config tree (config files
    /// require the dedicated ConfigWrite dimension on write).
    fn is_config_file(&self, path: &str) -> bool;
}
