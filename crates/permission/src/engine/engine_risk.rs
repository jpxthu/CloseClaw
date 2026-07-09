//! Permission Engine - Risk assessment
//!
//! Provides risk level classification for permission requests.

use crate::engine::engine_types::PermissionRequestBody;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::engine::engine_eval::PermissionEngine;
#[cfg(test)]
use crate::engine::engine_types::{Effect, PermissionRequest, PermissionResponse};
#[cfg(test)]
use crate::rules::RuleSetBuilder;

/// Risk level for permission requests.
/// Used to annotate denied responses with severity information.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Returns `true` if the risk level is `High` or `Critical`.
    pub fn is_high_or_critical(self) -> bool {
        matches!(self, RiskLevel::High | RiskLevel::Critical)
    }
}

/// A risk pattern with matching logic and associated risk level.
struct RiskPattern {
    /// Returns Some(RiskLevel) if the request matches, None otherwise.
    matches: fn(&PermissionRequestBody) -> Option<RiskLevel>,
}

/// Check if a path contains or ends with `.git`.
fn path_has_git(path: &str) -> bool {
    path.contains("/.git/") || path.ends_with("/.git") || path == ".git"
}

/// Check if this is a destructive `rm` command.
///
/// Matches `rm` with any flag combination that includes recursive semantics:
/// `-r`, `-R`, `-rf`, `-fr`, or `--recursive`. Force-only (`-f`) without
/// recursion is excluded because it does not perform recursive deletion.
fn is_destructive_rm(cmd: &str, args: &[String]) -> bool {
    if cmd != "rm" {
        return false;
    }
    args.iter()
        .any(|arg| matches!(arg.as_str(), "-r" | "-R" | "-rf" | "-fr" | "--recursive"))
}

const HIGH_RISK_PATTERNS: &[RiskPattern] = &[
    // .git path read/write → High
    RiskPattern {
        matches: |request| match request {
            PermissionRequestBody::FileOp { path, op, .. }
                if (op == "read" || op == "write") && path_has_git(path) =>
            {
                Some(RiskLevel::High)
            }
            _ => None,
        },
    },
    // permissions.json → Critical
    RiskPattern {
        matches: |request| match request {
            PermissionRequestBody::FileOp { path, op, .. }
                if (op == "read" || op == "write")
                    && (path.ends_with("permissions.json")
                        || path.contains("/permissions.json")) =>
            {
                Some(RiskLevel::Critical)
            }
            _ => None,
        },
    },
    // Template files in permission module → Critical
    RiskPattern {
        matches: |request| match request {
            PermissionRequestBody::FileOp { path, op, .. }
                if (op == "read" || op == "write")
                    && path.contains("/templates/")
                    && path.contains("permission") =>
            {
                Some(RiskLevel::Critical)
            }
            _ => None,
        },
    },
    // daemon/gateway config write → Critical
    RiskPattern {
        matches: |request| match request {
            PermissionRequestBody::ConfigWrite { config_file, .. }
                if config_file.contains("daemon") || config_file.contains("gateway") =>
            {
                Some(RiskLevel::Critical)
            }
            _ => None,
        },
    },
    // Destructive rm (with recursive flag) → High
    RiskPattern {
        matches: |request| match request {
            PermissionRequestBody::CommandExec { cmd, args, .. }
                if is_destructive_rm(cmd, args) =>
            {
                Some(RiskLevel::High)
            }
            _ => None,
        },
    },
];

/// Assess the risk level of a permission request.
///
/// Returns the highest matching risk level, or `RiskLevel::Low` if no patterns match.
pub fn assess_risk_level(request: &PermissionRequestBody) -> RiskLevel {
    for pattern in HIGH_RISK_PATTERNS {
        if let Some(level) = (pattern.matches)(request) {
            return level;
        }
    }
    RiskLevel::Low
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_path_file_read_returns_high() {
        let request = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "read".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_git_path_file_write_returns_high() {
        let request = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "src/.git/HEAD".to_string(),
            op: "write".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_permissions_json_returns_critical() {
        let request = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/etc/permissions.json".to_string(),
            op: "read".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Critical);
    }

    #[test]
    fn test_template_path_in_permission_module_returns_critical() {
        // Templates within permission module: path contains /templates/ AND has permission context
        let request = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/permissions/templates/admin_template.json".to_string(),
            op: "write".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Critical);
    }

    #[test]
    fn test_daemon_config_write_returns_critical() {
        let request = PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "daemon.json".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Critical);
    }

    #[test]
    fn test_destructive_rm_bare_rf_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_destructive_rm_f_r_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-f".to_string(), "-r".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_destructive_rm_rf_with_path_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/foo".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_destructive_rm_r_with_path_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-r".to_string(), "/important/path".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_rm_force_only_returns_low() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-f".to_string(), "file.txt".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    #[test]
    fn test_rm_no_flags_returns_low() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["file.txt".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    // ── is_destructive_rm edge cases ──────────────────────────────────────

    #[test]
    fn test_rm_recursive_long_flag_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["--recursive".to_string(), "/tmp/dir".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_rm_uppercase_r_with_path_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-R".to_string(), "/var/log".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_rm_fr_combined_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-fr".to_string(), "/home/user".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_rm_empty_args_returns_low() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec![],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    #[test]
    fn test_echo_rm_rf_not_matched() {
        // "echo" is the command, not "rm" — should not match.
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "echo".to_string(),
            args: vec!["rm".to_string(), "-rf".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    #[test]
    fn test_rm_recursive_force_long_flag_returns_high() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["--recursive".to_string(), "--force".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::High);
    }

    #[test]
    fn test_normal_file_returns_low() {
        let request = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "read".to_string(),
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    #[test]
    fn test_normal_command_returns_low() {
        let request = PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "ls".to_string(),
            args: vec!["-la".to_string()],
        };
        assert_eq!(assess_risk_level(&request), RiskLevel::Low);
    }

    #[test]
    fn test_risk_level_serde() {
        let level = RiskLevel::Critical;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, "\"critical\"");
        let parsed: RiskLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, RiskLevel::Critical);
    }

    #[test]
    fn test_risk_level_default() {
        let level = RiskLevel::default();
        assert_eq!(level, RiskLevel::Low);
    }

    // -------------------------------------------------------------------------
    // End-to-end risk level tests via PermissionEngine
    // -------------------------------------------------------------------------

    #[test]
    fn test_git_path_deny_risk_level_high() {
        let ruleset = RuleSetBuilder::new()
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new_with_default_data_root(ruleset);
        let resp = engine.evaluate(
            PermissionRequest::Bare(PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/repo/.git/config".to_string(),
                op: "read".to_string(),
            }),
            None,
        );
        match resp {
            PermissionResponse::Denied { risk_level, .. } => {
                assert_eq!(risk_level, RiskLevel::High);
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[test]
    fn test_normal_path_deny_risk_level_low() {
        let ruleset = RuleSetBuilder::new()
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new_with_default_data_root(ruleset);
        let resp = engine.evaluate(
            PermissionRequest::Bare(PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/repo/src/main.rs".to_string(),
                op: "read".to_string(),
            }),
            None,
        );
        match resp {
            PermissionResponse::Denied { risk_level, .. } => {
                assert_eq!(risk_level, RiskLevel::Low);
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }
}
