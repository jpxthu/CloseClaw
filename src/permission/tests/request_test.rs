//!
//! PermissionRequest envelope tests
//!

use crate::permission::engine::{Caller, PermissionRequest, PermissionRequestBody};

#[test]
fn test_bare_request_caller_defaults() {
    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/tmp".to_string(),
        op: "read".to_string(),
    });
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
    assert_eq!(caller.creator_id, "");
}

#[test]
fn test_with_caller_request() {
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        },
    };
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_bare_deserialize_old_format() {
    let json = r#"{"type": "file_op", "agent": "test-agent", "path": "/tmp", "op": "read"}"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(request, PermissionRequest::Bare(_)));
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
}

#[test]
fn test_with_caller_deserialize_new_format() {
    let json = r#"{
        "caller": {"user_id": "ou_alice", "agent": "dev-agent-01", "creator_id": ""},
        "type": "file_op",
        "agent": "dev-agent-01",
        "path": "/tmp",
        "op": "read"
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(request, PermissionRequest::WithCaller { .. }));
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_with_caller_deserialize_creator_id() {
    let json = r#"{
        "caller": {"user_id": "ou_john", "agent": "dev-agent-01", "creator_id": "ou_john"},
        "type": "command_exec",
        "agent": "dev-agent-01",
        "cmd": "rm",
        "args": ["-rf", "/"]
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_john");
    assert_eq!(caller.creator_id, "ou_john");
}

#[test]
fn test_with_caller_converts_bare() {
    let bare = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/tmp".to_string(),
        op: "read".to_string(),
    });
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let with_caller = bare.with_caller(caller);
    assert!(matches!(with_caller, PermissionRequest::WithCaller { .. }));
    assert_eq!(with_caller.caller().user_id, "ou_alice");
}

#[test]
fn test_permission_request_body_agent_id() {
    assert_eq!(
        PermissionRequestBody::FileOp {
            agent: "a".into(),
            path: "/".into(),
            op: "read".into()
        }
        .agent_id(),
        "a"
    );
    assert_eq!(
        PermissionRequestBody::InterAgentMsg {
            from: "a".into(),
            to: "b".into()
        }
        .agent_id(),
        "a"
    );
}
