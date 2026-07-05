use super::*;

fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
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

// --- intersect: all seven dimensions ---

#[test]
fn intersect_all_seven_dimensions_checked() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.permissions.len(), 7);
    for dim in [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ] {
        assert!(
            result.permissions.contains_key(dim),
            "missing dimension: {dim}"
        );
    }
}
