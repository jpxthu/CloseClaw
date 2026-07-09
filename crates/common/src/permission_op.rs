//! Permission management operations for slash commands.
//!
//! Defines [`PermissionOperation`] variants used by the `/perm` slash
//! command handler. The gateway intercepts [`crate::SlashResult::PermissionOp`]
//! and executes these operations directly in the daemon process without
//! entering an Agent Session.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// User registration types
// ---------------------------------------------------------------------------

/// Preset permission sets that an Owner can assign to a newly registered User.
///
/// Each variant maps to a concrete set of [`Rule`](crate) entries via
/// [`to_rules()`](InitialPermissionSet::to_rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialPermissionSet {
    /// Basic messaging: allows sending/receiving messages and reading the workspace.
    /// Translates to `ToolCall { skill: "chat", method: "send" }` + workspace read.
    BasicMessaging,
}

impl InitialPermissionSet {
    /// Human-readable name for display in confirmation messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::BasicMessaging => "BasicMessaging",
        }
    }
}

/// A registered user who has been approved by an Owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRegistration {
    /// Unique identifier for the user (e.g. Feishu `open_id`).
    pub user_id: String,
    /// IM channel through which the user interacts (e.g. "feishu").
    pub im_channel: String,
    /// Initial permission sets granted at registration time.
    pub initial_permissions: Vec<InitialPermissionSet>,
    /// ISO-8601 timestamp of when the user was registered.
    pub created_at: String,
}

/// Request payload carried through the [`ApprovalQueue`] for a new-user
/// registration that requires Owner approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreationRequest {
    /// The user who requested registration.
    pub user_id: String,
    /// IM channel the user will use.
    pub im_channel: String,
    /// Unique request identifier for tracking in the approval queue.
    pub request_id: String,
    /// Initial permission sets selected by the Owner at approval time.
    pub initial_permissions: Vec<InitialPermissionSet>,
}

/// A permission management operation, executed by the gateway/daemon.
///
/// Each variant corresponds to a `/perm` sub-command and carries the
/// parsed parameters needed to write a rule into `permissions.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionOperation {
    /// Add an allow rule for a file path pattern.
    AddFileWhitelist {
        /// Target agent identifier.
        agent: String,
        /// Operation type (e.g. "read", "write").
        op: String,
        /// File path patterns to whitelist.
        paths: Vec<String>,
    },
    /// Add a deny rule for a file path pattern.
    AddFileDeny {
        /// Target agent identifier.
        agent: String,
        /// Operation type (e.g. "read", "write").
        op: String,
        /// File path patterns to deny.
        paths: Vec<String>,
    },
    /// Add an allow rule for a command.
    AddCommandWhitelist {
        /// Target agent identifier.
        agent: String,
        /// Command name.
        command: String,
        /// Command arguments patterns.
        args: Vec<String>,
    },
    /// Add a deny rule for a command.
    AddCommandDeny {
        /// Target agent identifier.
        agent: String,
        /// Command name.
        command: String,
        /// Command arguments patterns.
        args: Vec<String>,
    },
    /// Register a new user with initial permissions.
    CreateUser {
        /// User identifier (e.g. Feishu `open_id`).
        user_id: String,
        /// IM channel the user will use (e.g. "feishu").
        channel: String,
        /// Initial permission sets to grant.
        initial_permissions: Vec<InitialPermissionSet>,
    },
}

impl PermissionOperation {
    /// Human-readable description of the operation for confirmation messages.
    pub fn describe(&self) -> String {
        match self {
            Self::AddFileWhitelist { agent, op, paths } => {
                format!(
                    "whitelist file {} for agent `{}`: {}",
                    op,
                    agent,
                    paths.join(", ")
                )
            }
            Self::AddFileDeny { agent, op, paths } => {
                format!(
                    "deny file {} for agent `{}`: {}",
                    op,
                    agent,
                    paths.join(", ")
                )
            }
            Self::AddCommandWhitelist {
                agent,
                command,
                args,
            } => {
                let full_cmd = if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args.join(" "))
                };
                format!("whitelist command `{}` for agent `{}`", full_cmd, agent)
            }
            Self::AddCommandDeny {
                agent,
                command,
                args,
            } => {
                let full_cmd = if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args.join(" "))
                };
                format!("deny command `{}` for agent `{}`", full_cmd, agent)
            }
            Self::CreateUser {
                user_id,
                channel,
                initial_permissions,
            } => {
                let perms: Vec<&str> = initial_permissions.iter().map(|p| p.label()).collect();
                format!(
                    "register user `{}` via {} with permissions [{}]",
                    user_id,
                    channel,
                    perms.join(", ")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_add_file_whitelist() {
        let op = PermissionOperation::AddFileWhitelist {
            agent: "eda".into(),
            op: "read".into(),
            paths: vec!["/tmp/data/**".into()],
        };
        assert_eq!(
            op.describe(),
            "whitelist file read for agent `eda`: /tmp/data/**"
        );
    }

    #[test]
    fn test_describe_add_file_denies() {
        let op = PermissionOperation::AddFileDeny {
            agent: "eda".into(),
            op: "write".into(),
            paths: vec!["/etc/**".into(), "/root/**".into()],
        };
        assert_eq!(
            op.describe(),
            "deny file write for agent `eda`: /etc/**, /root/**"
        );
    }

    #[test]
    fn test_describe_add_command_whitelist_no_args() {
        let op = PermissionOperation::AddCommandWhitelist {
            agent: "eda".into(),
            command: "ls".into(),
            args: vec![],
        };
        assert_eq!(op.describe(), "whitelist command `ls` for agent `eda`");
    }

    #[test]
    fn test_describe_add_command_whitelist_with_args() {
        let op = PermissionOperation::AddCommandWhitelist {
            agent: "eda".into(),
            command: "git".into(),
            args: vec!["status".into(), "log".into()],
        };
        assert_eq!(
            op.describe(),
            "whitelist command `git status log` for agent `eda`"
        );
    }

    #[test]
    fn test_describe_add_command_deny() {
        let op = PermissionOperation::AddCommandDeny {
            agent: "eda".into(),
            command: "rm".into(),
            args: vec!["-rf".into()],
        };
        assert_eq!(op.describe(), "deny command `rm -rf` for agent `eda`");
    }
}
