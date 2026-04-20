//!
//! Subject matching tests (UserAndAgent, AgentOnly) and JSON deserialization
//!

use crate::permission::engine::{Caller, MatchType, Subject};

// --- Subject::UserAndAgent tests ---

#[test]
fn test_user_and_agent_subject_matches_both_exact() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_user_mismatch() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_bob".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_agent_mismatch() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "other-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_glob_matching() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_admin_*".to_string(),
        agent: "dev-*".to_string(),
        user_match: MatchType::Glob,
        agent_match: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_admin_john".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_mixed_match_types() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_123".to_string(),
        agent: "dev-*".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_123".to_string(),
        agent: "dev-agent-99".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));

    let caller = Caller {
        user_id: "ou_456".to_string(),
        agent: "dev-agent-99".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

// --- Agent-only subject tests (backward compat) ---

#[test]
fn test_agent_only_subject_exact() {
    let subject = Subject::AgentOnly {
        agent: "dev-agent-01".to_string(),
        match_type: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_anyone".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_agent_only_subject_glob() {
    let subject = Subject::AgentOnly {
        agent: "dev-*".to_string(),
        match_type: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

// --- Subject JSON deserialization tests ---

#[test]
fn test_subject_deserialize_old_agent_only() {
    let json = r#"{"agent": "dev-agent-01", "match_type": "exact"}"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    assert!(matches!(subject, Subject::AgentOnly { .. }));
    assert_eq!(subject.agent_id(), "dev-agent-01");
}

#[test]
fn test_subject_deserialize_old_with_glob() {
    let json = r#"{"agent": "dev-*", "match": "glob"}"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    assert!(
        matches!(subject, Subject::AgentOnly { agent, match_type: MatchType::Glob } if agent == "dev-*")
    );
}

#[test]
fn test_subject_deserialize_new_user_and_agent() {
    let json = r#"{
        "match_mode": "user_and_agent",
        "user_id": "ou_alice",
        "agent": "dev-agent-01",
        "user_match": "exact",
        "agent_match": "exact"
    }"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    let Subject::UserAndAgent { user_id, agent, .. } = subject else {
        panic!("expected UserAndAgent")
    };
    assert_eq!(user_id, "ou_alice");
    assert_eq!(agent, "dev-agent-01");
}
