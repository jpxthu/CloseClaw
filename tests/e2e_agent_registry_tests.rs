//! E2E integration tests for Agent Registry lifecycle.
//! Uses `register` + `update_state` to simulate lifecycle (issue #494 degraded approach).

use closeclaw::agent::registry::create_registry;
use closeclaw::agent::{AgentState, ErrorInfo, SuspendedReason, TransitionTrigger};

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

/// Hierarchy queries: get_parent, get_ancestors, count, list_by_state
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
    let idle = registry.list_by_state(AgentState::Idle).await;
    assert_eq!(idle.len(), 3);
    let running = registry.list_by_state(AgentState::Running).await;
    assert!(running.is_empty());
}

/// State machine: Idle → Running → Suspended(Forced) → Running → Error(recoverable) → Running
#[tokio::test]
async fn test_state_machine_complete_transitions() {
    let registry = create_registry(30);
    let agent = registry.register("test".to_string(), None).await.unwrap();
    assert!(matches!(agent.state, AgentState::Idle));

    let agent = registry
        .update_state(
            &agent.id,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        )
        .await
        .unwrap();
    assert!(matches!(agent.state, AgentState::Running));

    let agent = registry
        .update_state(
            &agent.id,
            AgentState::Suspended(SuspendedReason::Forced),
            TransitionTrigger::Scheduler,
        )
        .await
        .unwrap();
    assert!(matches!(
        agent.state,
        AgentState::Suspended(SuspendedReason::Forced)
    ));

    let agent = registry
        .update_state(
            &agent.id,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        )
        .await
        .unwrap();
    assert!(matches!(agent.state, AgentState::Running));

    let agent = registry
        .update_state(
            &agent.id,
            AgentState::Error(ErrorInfo::new(" recoverable error", true)),
            TransitionTrigger::Error,
        )
        .await
        .unwrap();
    assert!(matches!(
        agent.state,
        AgentState::Error(ErrorInfo {
            recoverable: true,
            ..
        })
    ));

    let agent = registry
        .update_state(
            &agent.id,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        )
        .await
        .unwrap();
    assert!(matches!(agent.state, AgentState::Running));
}

/// Cascade stop: parent stop terminates all descendants
#[tokio::test]
async fn test_cascade_stop() {
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

    for id in [&root.id, &child.id, &grandchild.id] {
        registry
            .update_state(id, AgentState::Running, TransitionTrigger::UserRequest)
            .await
            .unwrap();
    }

    registry.stop_agent(&root.id, true).await.unwrap();

    assert!(matches!(
        registry.get(&root.id).await.unwrap().state,
        AgentState::Stopped
    ));
    assert!(matches!(
        registry.get(&child.id).await.unwrap().state,
        AgentState::Stopped
    ));
    assert!(matches!(
        registry.get(&grandchild.id).await.unwrap().state,
        AgentState::Stopped
    ));
}

/// Cascade suspend/resume: suspend_agent(cascade=true) + resume_agent
#[tokio::test]
async fn test_cascade_suspend_resume() {
    let registry = create_registry(30);
    let parent = registry.register("parent".to_string(), None).await.unwrap();
    let child = registry
        .register("child".to_string(), Some(parent.id.clone()))
        .await
        .unwrap();

    registry
        .update_state(
            &parent.id,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        )
        .await
        .unwrap();
    registry
        .update_state(
            &child.id,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        )
        .await
        .unwrap();

    registry
        .suspend_agent(&parent.id, SuspendedReason::Forced, true)
        .await
        .unwrap();

    assert!(matches!(
        registry.get(&parent.id).await.unwrap().state,
        AgentState::Suspended(SuspendedReason::Forced)
    ));
    assert!(matches!(
        registry.get(&child.id).await.unwrap().state,
        AgentState::Suspended(SuspendedReason::Forced)
    ));

    for agent_id in [parent.id.clone(), child.id.clone()] {
        registry.resume_agent(&agent_id).await.unwrap();
    }

    assert!(matches!(
        registry.get(&parent.id).await.unwrap().state,
        AgentState::Running
    ));
    assert!(matches!(
        registry.get(&child.id).await.unwrap().state,
        AgentState::Running
    ));
}

/// Terminal state and destroy: Stopped/Error(non-recoverable) cannot resume, destroy confirms
#[tokio::test]
async fn test_terminal_state_rejection_and_destroy() {
    let registry = create_registry(30);

    // Part A: Stopped cannot be resumed
    let a = registry.register("a".to_string(), None).await.unwrap();
    registry
        .update_state(&a.id, AgentState::Running, TransitionTrigger::UserRequest)
        .await
        .unwrap();
    registry.stop_agent(&a.id, false).await.unwrap();
    assert!(registry.resume_agent(&a.id).await.is_err());

    // Part B: Error(non-recoverable) cannot be resumed
    let b = registry.register("b".to_string(), None).await.unwrap();
    registry
        .update_state(&b.id, AgentState::Running, TransitionTrigger::UserRequest)
        .await
        .unwrap();
    registry
        .update_state(
            &b.id,
            AgentState::Error(ErrorInfo::new("non-recoverable", false)),
            TransitionTrigger::Error,
        )
        .await
        .unwrap();
    assert!(registry.resume_agent(&b.id).await.is_err());

    // Part C: destroy with confirmation
    let c = registry.register("c".to_string(), None).await.unwrap();
    let conf = registry.destroy_agent(&c.id, true).await.unwrap();
    let token = conf.unwrap().confirm_token;
    assert!(registry.get(&c.id).await.is_ok());
    registry.confirm_destroy(&c.id, &token).await.unwrap();
    assert!(registry.get(&c.id).await.is_err());
}
