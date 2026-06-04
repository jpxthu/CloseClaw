//! E2E integration tests for Agent Registry lifecycle.
//! Focused on hierarchy, register/get/list/remove/get_children/get_descendants
//! after the Step 1.3 removal of the state machine and cascade APIs.

use closeclaw::agent::registry::create_registry;

/// Parent-child registration: root -> child -> grandchild
#[tokio::test]
async fn test_parent_child_registration_chain() {
    let registry = create_registry(30);
    let root = registry.register("root".to_string(), None).await.unwrap();
    let child = registry
        .register("child".to_string(), Some(root.id.clone()))
        .await
        .unwrap();
    let grandchild = registry
        .register("grandchild".to_string(), Some(child.id.clone()))
        .await
        .unwrap();

    assert!(root.parent_id.is_none());
    assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
    assert_eq!(grandchild.parent_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(registry.get_children(&root.id).await.len(), 1);
    assert_eq!(registry.get_children(&root.id).await[0].id, child.id);
    assert_eq!(registry.get_children(&child.id).await.len(), 1);
    assert_eq!(registry.get_children(&child.id).await[0].id, grandchild.id);
    assert!(registry.get_children(&grandchild.id).await.is_empty());
}

/// Hierarchy queries: get_children, is_ancestor_of, get_descendants
#[tokio::test]
async fn test_hierarchy_get_children() {
    let registry = create_registry(30);
    let root = registry.register("root".to_string(), None).await.unwrap();
    let child = registry
        .register("child".to_string(), Some(root.id.clone()))
        .await
        .unwrap();
    registry
        .register("grandchild".to_string(), Some(child.id.clone()))
        .await
        .unwrap();

    let children = registry.get_children(&root.id).await;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "child");

    let descendants = registry.get_descendants(&root.id).await;
    assert_eq!(descendants.len(), 2);
    let names: Vec<_> = descendants.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"child"));
    assert!(names.contains(&"grandchild"));

    assert!(registry.is_ancestor_of(&root.id, &child.id).await);
    assert!(
        registry
            .is_ancestor_of(&root.id, &registry.get_children(&root.id).await[0].id)
            .await
    );
}

/// Hierarchy queries: get_parent, get_ancestors, count, list()
#[tokio::test]
async fn test_hierarchy_get_ancestors() {
    let registry = create_registry(30);
    let root = registry.register("root".to_string(), None).await.unwrap();
    let child = registry
        .register("child".to_string(), Some(root.id.clone()))
        .await
        .unwrap();
    let grandchild = registry
        .register("grandchild".to_string(), Some(child.id.clone()))
        .await
        .unwrap();

    let parent = registry.get_parent(&child.id).await;
    assert_eq!(parent.unwrap().id, root.id);
    assert!(registry.get_parent(&root.id).await.is_none());

    let ancestors = registry.get_ancestors(&grandchild.id).await;
    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0].name, "child");
    assert_eq!(ancestors[1].name, "root");

    assert_eq!(registry.count().await, 3);
    // Step 1.1 removed `Agent::state`; list() now returns the plain
    // hierarchy without state filtering.
    let all = registry.list().await;
    assert_eq!(all.len(), 3);
}

// Note: `test_state_machine_complete_transitions`, `test_cascade_stop`,
// `test_cascade_suspend_resume`, and `test_terminal_state_rejection_and_destroy`
// were removed in Step 1.3 along with the `update_state` / cascade APIs.
// The `AgentState` machine itself still lives in `crate::agent::state` and
// is exercised by its own unit tests.
