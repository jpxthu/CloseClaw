//! Unit tests for permission_admission payload types.

use super::*;
use crate::permission_types::RiskLevel;

// ---------------------------------------------------------------------------
// ExecLocation / MessageDirection
// ---------------------------------------------------------------------------

#[test]
fn test_exec_location_default_is_sandbox() {
    assert_eq!(ExecLocation::default(), ExecLocation::Sandbox);
}

#[test]
fn test_exec_location_serde_roundtrip() {
    for (value, json) in [
        (ExecLocation::Sandbox, "\"sandbox\""),
        (ExecLocation::Host, "\"host\""),
    ] {
        assert_eq!(serde_json::to_string(&value).unwrap(), json);
        let parsed: ExecLocation = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, value);
    }
}

#[test]
fn test_message_direction_serde_roundtrip() {
    for (value, json) in [
        (MessageDirection::Inbound, "\"inbound\""),
        (MessageDirection::Outbound, "\"outbound\""),
    ] {
        assert_eq!(serde_json::to_string(&value).unwrap(), json);
        let parsed: MessageDirection = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, value);
    }
}

// ---------------------------------------------------------------------------
// PermissionOperation
// ---------------------------------------------------------------------------

#[test]
fn test_permission_operation_file_variants_serde_roundtrip() {
    let read = PermissionOperation::FileRead {
        path: "/workspace/report.md".into(),
    };
    let json = serde_json::to_string(&read).unwrap();
    assert_eq!(json, r#"{"file_read":{"path":"/workspace/report.md"}}"#);
    let parsed: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, read);

    let write = PermissionOperation::FileWrite {
        path: "/workspace/out.txt".into(),
    };
    let parsed: PermissionOperation =
        serde_json::from_str(&serde_json::to_string(&write).unwrap()).unwrap();
    assert_eq!(parsed, write);

    let config = PermissionOperation::ConfigWrite {
        path: "/workspace/closeclaw.toml".into(),
    };
    let parsed: PermissionOperation =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(parsed, config);
}

#[test]
fn test_permission_operation_command_serde_roundtrip() {
    let op = PermissionOperation::Command {
        command: "cargo build".into(),
        location: ExecLocation::Host,
    };
    let json = serde_json::to_string(&op).unwrap();
    assert_eq!(
        json,
        r#"{"command":{"command":"cargo build","location":"host"}}"#
    );
    let parsed: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, op);
}

#[test]
fn test_permission_operation_network_serde_roundtrip() {
    let op = PermissionOperation::Network {
        host: "api.example.com".into(),
        port: 443,
    };
    let json = serde_json::to_string(&op).unwrap();
    assert_eq!(json, r#"{"network":{"host":"api.example.com","port":443}}"#);
    let parsed: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, op);
}

#[test]
fn test_permission_operation_tool_call_serde_roundtrip() {
    let op = PermissionOperation::ToolCall {
        tool_name: "web_search".into(),
    };
    let json = serde_json::to_string(&op).unwrap();
    assert_eq!(json, r#"{"tool_call":{"tool_name":"web_search"}}"#);
    let parsed: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, op);
}

#[test]
fn test_permission_operation_user_message_serde_roundtrip() {
    let op = PermissionOperation::UserMessage {
        direction: MessageDirection::Outbound,
    };
    let json = serde_json::to_string(&op).unwrap();
    assert_eq!(json, r#"{"user_message":{"direction":"outbound"}}"#);
    let parsed: PermissionOperation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, op);
}

// ---------------------------------------------------------------------------
// PermissionRequest
// ---------------------------------------------------------------------------

#[test]
fn test_permission_request_serde_roundtrip_with_optional_fields() {
    let request = PermissionRequest {
        agent_id: "agent-main".into(),
        user_id: "ou_caller".into(),
        is_sub_session: false,
        session_id: Some("sess-001".into()),
        operation: PermissionOperation::Command {
            command: "rm -rf build".into(),
            location: ExecLocation::Host,
        },
        purpose: Some("clean stale build output".into()),
    };
    let json = serde_json::to_string(&request).unwrap();
    let parsed: PermissionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, request);
    assert!(json.contains("\"session_id\":\"sess-001\""));
    assert!(json.contains("\"purpose\":\"clean stale build output\""));
}

#[test]
fn test_permission_request_none_option_fields_roundtrip() {
    let request = PermissionRequest {
        agent_id: "agent-main".into(),
        user_id: "ou_caller".into(),
        is_sub_session: true,
        session_id: None,
        operation: PermissionOperation::FileRead {
            path: "/workspace/notes.md".into(),
        },
        purpose: None,
    };
    let json = serde_json::to_string(&request).unwrap();
    let parsed: PermissionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, request);
    assert_eq!(parsed.session_id, None);
    assert_eq!(parsed.purpose, None);
    assert!(json.contains("\"session_id\":null"));
    assert!(json.contains("\"purpose\":null"));
}

// ---------------------------------------------------------------------------
// ExecutionPolicy
// ---------------------------------------------------------------------------

#[test]
fn test_execution_policy_default_is_sandbox_with_empty_sets() {
    let policy = ExecutionPolicy::default();
    assert_eq!(policy.location, ExecLocation::Sandbox);
    assert!(policy.read_paths.is_empty());
    assert!(policy.write_paths.is_empty());
    assert!(policy.deny_paths.is_empty());
    assert!(policy.network_targets.is_empty());
}

#[test]
fn test_execution_policy_serde_roundtrip() {
    let policy = ExecutionPolicy {
        location: ExecLocation::Host,
        read_paths: vec!["/workspace".into()],
        write_paths: vec![],
        deny_paths: vec!["/workspace/secrets".into()],
        network_targets: vec!["api.example.com:443".into()],
    };
    let json = serde_json::to_string(&policy).unwrap();
    let parsed: ExecutionPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, policy);
    assert!(json.contains("\"location\":\"host\""));
    assert!(json.contains("\"write_paths\":[]"));
}

// ---------------------------------------------------------------------------
// PermissionDecision
// ---------------------------------------------------------------------------

#[test]
fn test_permission_decision_allowed_with_policy_roundtrip() {
    let decision = PermissionDecision::Allowed {
        policy: Some(ExecutionPolicy {
            location: ExecLocation::Sandbox,
            read_paths: vec!["/workspace".into()],
            write_paths: vec!["/workspace/out".into()],
            deny_paths: vec![],
            network_targets: vec![],
        }),
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: PermissionDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, decision);
    assert!(json.contains("\"allowed\""));
    assert!(json.contains("\"policy\""));
}

#[test]
fn test_permission_decision_allowed_without_policy_roundtrip() {
    let decision = PermissionDecision::Allowed { policy: None };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: PermissionDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, decision);
    assert!(json.contains("\"policy\":null"));
}

#[test]
fn test_permission_decision_denied_roundtrip() {
    let decision = PermissionDecision::Denied {
        reason: "operation denied by permission rules".into(),
        risk_level: RiskLevel::High,
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: PermissionDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, decision);
    assert!(json.contains("\"denied\""));
    assert!(json.contains("\"risk_level\":\"high\""));
}
