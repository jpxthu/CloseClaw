use super::*;

// =====================================================================
// Step 1.5 — intersect() + is_fully_denied() tests
// =====================================================================

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
            let allowed = allowed_dims.contains(&dim);
            (
                dim.to_string(),
                ActionPermission {
                    allowed,
                    limits: if allowed {
                        PermissionLimits {
                            commands: vec![],
                            paths: vec![],
                            timeout_ms: None,
                        }
                    } else {
                        PermissionLimits::default()
                    },
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

#[test]
fn test_intersect_both_allow() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(result.permissions.get("exec").unwrap().allowed);
    assert!(result.permissions.get("file_read").unwrap().allowed);
}

#[test]
fn test_intersect_child_deny() {
    let child = make_perms("child", &["file_read"]); // exec denied in child
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions.get("exec").unwrap().allowed);
    assert!(result.permissions.get("file_read").unwrap().allowed);
}

#[test]
fn test_intersect_parent_deny() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec"]); // file_read denied in parent
    let result = child.intersect(&parent);
    assert!(result.permissions.get("exec").unwrap().allowed);
    assert!(!result.permissions.get("file_read").unwrap().allowed);
}

#[test]
fn test_intersect_absent_is_deny() {
    // child has empty permissions map (no dimensions at all)
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    let parent = make_perms("parent", &["exec"]); // exec is allowed in parent
    let result = child.intersect(&parent);
    // absent in child → deny
    assert!(!result.permissions.get("exec").unwrap().allowed);
}

#[test]
fn test_intersect_result_identity() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.agent_id, "child");
    assert_eq!(result.inherited_from, Some("parent".to_string()));
}

#[test]
fn test_intersect_limits_commands_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["git".to_string(), "ls".to_string()],
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
                    commands: vec!["git".to_string(), "cat".to_string()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    let cmds = &result.permissions.get("exec").unwrap().limits.commands;
    assert_eq!(cmds, &vec!["git".to_string()]);
}

#[test]
fn test_intersect_limits_paths_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "file_read".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    paths: vec!["/data/**".to_string(), "/home/**".to_string()],
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
                    paths: vec!["/data/**".to_string(), "/etc/**".to_string()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    let paths = &result.permissions.get("file_read").unwrap().limits.paths;
    assert_eq!(paths, &vec!["/data/**".to_string()]);
}

#[test]
fn test_intersect_limits_timeout_min() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(60000),
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
                    timeout_ms: Some(30000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(
        result.permissions.get("exec").unwrap().limits.timeout_ms,
        Some(30000)
    );
}

#[test]
fn test_intersect_limits_none_no_restriction() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    commands: vec![],
                    paths: vec![],
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
                    timeout_ms: Some(5000),
                    commands: vec![],
                    paths: vec![],
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    // None (child) vs Some(5000) (parent) → Some(5000)
    assert_eq!(
        result.permissions.get("exec").unwrap().limits.timeout_ms,
        Some(5000)
    );
}

#[test]
fn test_is_fully_denied_true() {
    let perms = make_perms("agent", &[]); // no dimensions allowed
    assert!(perms.is_fully_denied());
}

#[test]
fn test_is_fully_denied_false() {
    let perms = make_perms("agent", &["exec"]); // one dimension allowed
    assert!(!perms.is_fully_denied());
}

#[test]
fn test_is_fully_denied_empty() {
    let perms = AgentPermissions {
        agent_id: "agent".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    assert!(perms.is_fully_denied());
}
