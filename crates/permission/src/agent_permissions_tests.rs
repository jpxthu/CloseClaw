//! UT for the permission-owned agent permission types.
//!
//! The intersect / is_fully_denied suite is ported 1:1 from the config-side
//! suite (`crates/config/src/agents/config_types/tests.rs`) — identical
//! fixtures and assertions prove semantic equivalence of the migrated
//! implementation.

use crate::agent_permissions::{
    intersect_option_min, intersect_vec, ActionPermission, AgentPermissionProvider,
    AgentPermissions, PermissionLimits,
};
use std::collections::HashMap;

fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
    ];
    let permissions = dimensions
        .iter()
        .map(|&dim| {
            (
                dim.to_string(),
                ActionPermission {
                    allowed: allowed_dims.contains(&dim),
                    limits: PermissionLimits::default(),
                },
            )
        })
        .collect();
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

// --- is_allowed ---

#[test]
fn is_allowed_present_allow_true_present_deny_absent_false() {
    let perms = make_perms("a", &["exec"]);
    assert!(perms.is_allowed("exec"));
    assert!(!perms.is_allowed("network"));
    assert!(!perms.is_allowed("message"));
}

// --- intersect: normal path ---

#[test]
fn intersect_both_allow_preserves() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(result.permissions["exec"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_limits_commands_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["git".into(), "ls".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["git".into(), "cat".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.commands, vec!["git"]);
}

#[test]
fn intersect_limits_paths_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "file_read".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    paths: vec!["/data/**".into(), "/home/**".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "file_read".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    paths: vec!["/data/**".into(), "/etc/**".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(
        result.permissions["file_read"].limits.paths,
        vec!["/data/**"]
    );
}

#[test]
fn intersect_limits_timeout_min() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(60_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(30_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, Some(30_000));
}

// --- intersect: error path ---

#[test]
fn intersect_child_deny_overrides() {
    let child = make_perms("child", &["file_read"]); // exec denied
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["exec"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_parent_deny_overrides() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec"]); // file_read denied
    let result = child.intersect(&parent);
    assert!(result.permissions["exec"].allowed);
    assert!(!result.permissions["file_read"].allowed);
}

#[test]
fn intersect_absent_dimension_is_deny() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["exec"].allowed);
}

#[test]
fn intersect_deny_gets_default_limits() {
    let child = make_perms("child", &[]); // all denied
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    let exec = &result.permissions["exec"].limits;
    assert!(exec.commands.is_empty());
    assert!(exec.paths.is_empty());
    assert_eq!(exec.timeout_ms, None);
}

// --- intersect: boundary values ---

#[test]
fn intersect_limits_timeout_none_vs_none() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, None);
}

#[test]
fn intersect_limits_timeout_none_vs_some() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(5_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, Some(5_000));
}

#[test]
fn intersect_vec_none_none_returns_empty() {
    let result = intersect_vec::<String>(None, None);
    assert!(result.is_empty());
}

#[test]
fn intersect_vec_some_none_returns_some() {
    let a = vec!["x".to_string(), "y".to_string()];
    let result = intersect_vec(Some(&a), None);
    assert_eq!(result, vec!["x", "y"]);
}

#[test]
fn intersect_vec_none_some_returns_some() {
    let b = vec!["x".to_string(), "z".to_string()];
    let result = intersect_vec(None, Some(&b));
    assert_eq!(result, vec!["x", "z"]);
}

#[test]
fn intersect_option_min_both_some_takes_min() {
    assert_eq!(
        intersect_option_min(Some(60_000), Some(30_000)),
        Some(30_000)
    );
    assert_eq!(
        intersect_option_min(Some(30_000), Some(60_000)),
        Some(30_000)
    );
}

#[test]
fn intersect_option_min_one_none_takes_other() {
    assert_eq!(intersect_option_min(Some(5_000), None), Some(5_000));
    assert_eq!(intersect_option_min(None, Some(5_000)), Some(5_000));
}

#[test]
fn intersect_option_min_none_none_is_none() {
    assert_eq!(intersect_option_min(None, None), None);
}

// --- state transition: is_fully_denied ---

#[test]
fn is_fully_denied_all_absent() {
    let perms = AgentPermissions {
        agent_id: "a".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    assert!(perms.is_fully_denied());
}

#[test]
fn is_fully_denied_all_explicit_deny() {
    let perms = make_perms("a", &[]);
    assert!(perms.is_fully_denied());
}

#[test]
fn is_fully_denied_one_allow() {
    let perms = make_perms("a", &["exec"]);
    assert!(!perms.is_fully_denied());
}

// --- intersect: result identity ---

#[test]
fn intersect_result_has_correct_ids() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.agent_id, "child");
    assert_eq!(result.inherited_from, Some("parent".into()));
}

// --- intersect: all eight dimensions ---

#[test]
fn intersect_all_eight_dimensions_checked() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.permissions.len(), 8);
    for dim in [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
    ] {
        assert!(
            result.permissions.contains_key(dim),
            "missing dimension: {dim}"
        );
    }
}

// --- intersect: per-dimension sweep (all eight dimensions) ---

#[test]
fn intersect_deny_and_absent_rules_hold_for_each_dimension() {
    for dim in [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
    ] {
        // allow ∩ deny = deny
        let child = make_perms("child", &[]);
        let parent = make_perms("parent", &[dim]);
        assert!(
            !child.intersect(&parent).permissions[dim].allowed,
            "allow ∩ deny must be deny for dimension: {dim}"
        );
        // child allow ∩ parent absent = deny
        let child = make_perms("child", &[dim]);
        let parent = AgentPermissions {
            agent_id: "parent".to_string(),
            permissions: HashMap::new(),
            inherited_from: None,
        };
        assert!(
            !child.intersect(&parent).permissions[dim].allowed,
            "allow ∩ absent must be deny for dimension: {dim}"
        );
        // allow ∩ allow = allow
        let child = make_perms("child", &[dim]);
        let parent = make_perms("parent", &[dim]);
        assert!(
            child.intersect(&parent).permissions[dim].allowed,
            "allow ∩ allow must be allow for dimension: {dim}"
        );
    }
}

// --- intersect: message dimension ---

#[test]
fn intersect_parent_deny_child_allow_message_is_deny() {
    let child = make_perms("child", &["exec", "file_read", "message"]);
    let parent = make_perms("parent", &["exec", "file_read"]); // message absent = deny
    let result = child.intersect(&parent);
    assert!(!result.permissions["message"].allowed);
}

#[test]
fn intersect_both_allow_message_is_allow() {
    let child = make_perms("child", &["exec", "message"]);
    let parent = make_perms("parent", &["exec", "message"]);
    let result = child.intersect(&parent);
    assert!(result.permissions["message"].allowed);
}

#[test]
fn intersect_child_allow_parent_absent_message_is_deny() {
    let child = make_perms("child", &["exec", "message"]);
    // parent has no message dimension → absent treated as deny
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits::default(),
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert!(!result.permissions["message"].allowed);
}

// --- is_fully_denied: message dimension ---

#[test]
fn is_fully_denied_seven_deny_message_allow_is_false() {
    // All 7 non-message dimensions deny, message allow → not fully denied
    let perms = make_perms("a", &["message"]);
    assert!(!perms.is_fully_denied());
}

#[test]
fn is_fully_denied_all_eight_deny_is_true() {
    let perms = make_perms("a", &[]);
    assert!(perms.is_fully_denied());
}

// --- intersect: message dimension limits ---

#[test]
fn intersect_message_limits_commands_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "message".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["send".into(), "edit".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "message".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["send".into(), "delete".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert!(result.permissions["message"].allowed);
    assert_eq!(result.permissions["message"].limits.commands, vec!["send"]);
}

// --- AgentPermissionProvider port ---

struct MapProvider {
    inner: HashMap<String, AgentPermissions>,
}

impl AgentPermissionProvider for MapProvider {
    fn get(&self, agent_id: &str) -> Option<AgentPermissions> {
        self.inner.get(agent_id).cloned()
    }
}

#[test]
fn provider_trait_dispatches_by_agent_id() {
    let mut inner = HashMap::new();
    inner.insert("child".to_string(), make_perms("child", &["exec"]));
    let provider = MapProvider { inner };
    let provider: &dyn AgentPermissionProvider = &provider;
    let perms = provider.get("child").expect("child should resolve");
    assert_eq!(perms.agent_id, "child");
    assert!(provider.get("unknown").is_none());
}

#[test]
fn provider_trait_object_is_send_sync() {
    fn assert_bounds<T: Send + Sync>() {}
    assert_bounds::<Box<dyn AgentPermissionProvider>>();
}
