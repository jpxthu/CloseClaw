use super::*;

// AgentConfig / permissions / ModelSpec serde tests and the
// `from_single` / `merge` construction tests moved to the config crate
// (issue #3344): the agent crate no longer depends on `closeclaw-config`.

#[test]
fn test_default_communication_config() {
    // CommunicationConfig is still available as a standalone type.
    let with_parent = CommunicationConfig::default_with_parent(Some("parent-1"));
    assert_eq!(with_parent.outbound, vec!["parent-1"]);
    assert_eq!(with_parent.inbound, vec!["parent-1"]);

    let without_parent = CommunicationConfig::default_with_parent(None);
    assert!(without_parent.outbound.is_empty());
    assert!(without_parent.inbound.is_empty());
}

#[test]
fn test_communication_allowed() {
    // CommunicationConfig and check_communication_allowed still work as
    // standalone functions, even though AgentConfig no longer has a
    // communication field (removed in an earlier design-doc alignment
    // round - not in design doc).
    let source_comm = CommunicationConfig {
        outbound: vec!["child-1".to_string()],
        inbound: vec!["child-1".to_string()],
    };

    let target_comm = CommunicationConfig::default_with_parent(Some("parent-1"));

    // Parent -> Child should be allowed
    let result = check_communication_allowed(&source_comm, "parent-1", &target_comm, "child-1");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // Child -> Parent should be allowed
    let result = check_communication_allowed(&target_comm, "child-1", &source_comm, "parent-1");
    assert_eq!(result, CommunicationCheckResult::Allowed);
}

#[test]
fn test_communication_denied_outbound() {
    let agent_a_comm = CommunicationConfig {
        outbound: vec!["agent-b".to_string()],
        inbound: vec!["agent-b".to_string()],
    };

    let agent_c_comm = CommunicationConfig {
        outbound: vec![],
        inbound: vec![],
    };

    // Agent A -> Agent C: A's outbound doesn't contain C
    let result = check_communication_allowed(&agent_a_comm, "agent-a", &agent_c_comm, "agent-c");
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
}

#[test]
fn test_communication_denied_inbound() {
    let agent_a_comm = CommunicationConfig {
        outbound: vec!["agent-b".to_string()],
        inbound: vec!["agent-b".to_string()],
    };

    let agent_b_comm = CommunicationConfig {
        outbound: vec![],
        inbound: vec![], // B doesn't accept inbound from anyone
    };

    // Agent A -> Agent B: A's outbound contains B, but B's inbound doesn't contain A
    let result = check_communication_allowed(&agent_a_comm, "agent-a", &agent_b_comm, "agent-b");
    assert_eq!(result, CommunicationCheckResult::SourceNotInTargetInbound);
}
