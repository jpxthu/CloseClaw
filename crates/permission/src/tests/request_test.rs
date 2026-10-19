//!
//! PermissionRequest envelope tests
//!

use crate::engine::{Caller, PermissionRequest, PermissionRequestBody};
use tempfile::TempDir;

#[test]
fn test_bare_request_caller_defaults() {
    let tmp = TempDir::new().unwrap();
    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: tmp.path().to_string_lossy().into_owned(),
        op: "read".to_string(),
    });
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
}

#[test]
fn test_with_caller_request() {
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        },
        owner_id: None,
    };
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_bare_deserialize_old_format() {
    let tmp = TempDir::new().unwrap();
    let json = format!(
        r#"{{"type": "file_op", "agent": "test-agent", "path": "{}", "op": "read"}}"#,
        tmp.path().display()
    );
    let request: PermissionRequest = serde_json::from_str(&json).unwrap();
    assert!(matches!(request, PermissionRequest::Bare(_)));
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
}

#[test]
fn test_with_caller_deserialize_new_format() {
    let tmp = TempDir::new().unwrap();
    let json = format!(
        r#"{{
        "caller": {{"user_id": "ou_alice", "agent": "dev-agent-01"}},
        "type": "file_op",
        "agent": "dev-agent-01",
        "path": "{}",
        "op": "read"
    }}"#,
        tmp.path().display()
    );
    let request: PermissionRequest = serde_json::from_str(&json).unwrap();
    assert!(matches!(request, PermissionRequest::WithCaller { .. }));
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_with_caller_deserialize() {
    let json = r#"{
        "caller": {"user_id": "ou_john", "agent": "dev-agent-01"},
        "type": "command_exec",
        "agent": "dev-agent-01",
        "cmd": "rm",
        "args": ["-rf", "/"]
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_john");
}

#[test]
fn test_with_caller_converts_bare() {
    let tmp = TempDir::new().unwrap();
    let bare = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: tmp.path().to_string_lossy().into_owned(),
        op: "read".to_string(),
    });
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "test-agent".to_string(),
    };
    let with_caller = bare.with_caller(caller);
    assert!(matches!(with_caller, PermissionRequest::WithCaller { .. }));
    assert_eq!(with_caller.caller().user_id, "ou_alice");
}

// ── Session-bound Owner id channel ──────────────────────────────────────────

#[test]
fn test_with_caller_deserialize_without_owner_id_defaults_none() {
    // Legacy payload without the owner_id field must keep deserializing
    // (serde `Option` default) — old callers are not broken.
    let json = r#"{
        "caller": {"user_id": "ou_alice", "agent": "dev-agent-01"},
        "type": "file_op",
        "agent": "dev-agent-01",
        "path": "/any/path.txt",
        "op": "read"
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(request, PermissionRequest::WithCaller { .. }));
    assert_eq!(request.owner_id(), None);
}

#[test]
fn test_owner_id_serde_round_trip() {
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: Some("ou_alice".to_string()),
    };
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"owner_id\":\"ou_alice\""));
    let deserialized: PermissionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.owner_id(), Some("ou_alice"));
}

#[test]
fn test_with_owner_id_builder_sets_and_replaces() {
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: None,
    };
    assert_eq!(request.owner_id(), None);
    let bound = request.with_owner_id(Some("ou_alice".to_string()));
    assert_eq!(bound.owner_id(), Some("ou_alice"));
    let cleared = bound.with_owner_id(None);
    assert_eq!(cleared.owner_id(), None);
}

#[test]
fn test_bare_request_owner_id_is_none() {
    let bare = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    });
    assert_eq!(bare.owner_id(), None);
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
