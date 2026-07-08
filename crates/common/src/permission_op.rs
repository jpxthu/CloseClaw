//! Permission management operations for slash commands.
//!
//! Defines [`PermissionOperation`] variants used by the `/perm` slash
//! command handler. The gateway intercepts [`crate::SlashResult::PermissionOp`]
//! and executes these operations directly in the daemon process without
//! entering an Agent Session.

use serde::{Deserialize, Serialize};

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
