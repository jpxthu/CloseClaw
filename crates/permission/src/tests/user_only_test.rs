//! Tests for UserOnly subject variant (Step 1.6).
//!
//! Covers the behavior dimensions specified in the plan:
//! - **Normal path**: UserOnly rule matches same user_id across different agents
//! - **Normal path**: UserOnly rule does not match different user_id
//! - **Edge case**: user_id empty → fallback to AgentOnly
//! - **Edge case**: UserOnly + UserAndAgent coexistence
//! - **State transition**: Owner approval with --user-only generates correct rule
//! - **Error path**: UserOnly rule in Owner shortcut (skipped)

use crate::approval::WhitelistTarget;
use crate::engine::engine_eval::PermissionEngine;
use crate::engine::engine_types::{
    Action, Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Subject,
};
use crate::rules::{RuleBuilder, RuleSetBuilder};
use crate::whitelist::{build_whitelist_rule, caller_to_subject};

// ── helpers ──────────────────────────────────────────────────────────────────

fn file_read_request(agent: &str, user_id: &str, creator_id: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: agent.to_string(),
            creator_id: creator_id.to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: agent.to_string(),
            path: "/data/readme.txt".to_string(),
            op: "read".to_string(),
        },
    }
}

fn file_write_request(agent: &str, user_id: &str, creator_id: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: agent.to_string(),
            creator_id: creator_id.to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: agent.to_string(),
            path: "/data/output.txt".to_string(),
            op: "write".to_string(),
        },
    }
}

// ── Normal path: UserOnly matches same user_id across different agents ──────

#[tokio::test]
async fn test_user_only_matches_same_user_across_agents() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-read-allow")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Agent "alpha" — alice should be allowed
    let resp = engine.evaluate(file_read_request("alpha", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "UserOnly allow should match alice on agent alpha, got: {resp:?}"
    );

    // Agent "beta" — alice should also be allowed (UserOnly is agent-agnostic)
    let resp = engine.evaluate(file_read_request("beta", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "UserOnly allow should match alice on agent beta, got: {resp:?}"
    );
}

#[tokio::test]
async fn test_user_only_deny_matches_same_user_across_agents() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-write-deny")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .deny()
                .action(Action::File {
                    operation: "write".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_write(Effect::Allow)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Alice write should be denied on any agent
    let resp = engine.evaluate(file_write_request("any-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly deny should block alice write on any agent, got: {resp:?}"
    );
}

// ── Normal path: UserOnly does not match different user_id ───────────────────

#[tokio::test]
async fn test_user_only_no_match_different_user() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-read-allow")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Bob should NOT be affected by alice's rule
    let resp = engine.evaluate(file_read_request("any-agent", "ou_bob", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly alice rule should not match bob, got: {resp:?}"
    );
}

#[tokio::test]
async fn test_user_only_no_match_empty_user() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-read-allow")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Empty user_id should not match alice's UserOnly rule
    let resp = engine.evaluate(file_read_request("any-agent", "", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly alice rule should not match empty user_id, got: {resp:?}"
    );
}

// ── Edge case: user_id empty → fallback to AgentOnly ─────────────────────────

#[tokio::test]
async fn test_caller_to_subject_user_only_empty_user_fallback() {
    let caller = Caller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let subject = caller_to_subject(&caller, WhitelistTarget::UserOnly);
    assert!(
        subject.is_agent_only(),
        "UserOnly with empty user_id should fallback to AgentOnly"
    );
    assert_eq!(subject.agent_id(), "test-agent");
}

#[tokio::test]
async fn test_caller_to_subject_user_only_non_empty_user() {
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let subject = caller_to_subject(&caller, WhitelistTarget::UserOnly);
    assert!(
        subject.is_user_only(),
        "UserOnly with non-empty user_id should produce UserOnly"
    );
    assert_eq!(subject.user_id(), "ou_alice");
    assert_eq!(subject.agent_id(), "");
}

// ── Edge case: UserOnly + UserAndAgent coexistence ───────────────────────────

#[tokio::test]
async fn test_user_only_and_user_and_agent_coexist() {
    let ruleset = RuleSetBuilder::new()
        // UserOnly: alice can read on any agent (independent of agent phase)
        .rule(
            RuleBuilder::new()
                .name("alice-all-agents-read")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        // UserAndAgent: alice can write on dev-agent (requires agent phase agreement)
        .rule(
            RuleBuilder::new()
                .name("alice-specific-agent-write")
                .subject(Subject::UserAndAgent {
                    user_id: "ou_alice".to_string(),
                    agent: "dev-agent".to_string(),
                    user_match: MatchType::Exact,
                    agent_match: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "write".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        // AgentOnly: dev-agent allows write (so UserAndAgent has agent-phase agreement)
        .rule(
            RuleBuilder::new()
                .name("dev-agent-write-allow")
                .subject_agent("dev-agent")
                .allow()
                .action(Action::File {
                    operation: "write".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Alice read on other-agent — UserOnly matches (no agent rule needed)
    let resp = engine.evaluate(file_read_request("other-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "UserOnly should match alice read on other-agent, got: {resp:?}"
    );

    // Alice write on dev-agent — AgentOnly + UserAndAgent both match → Allowed
    let resp = engine.evaluate(file_write_request("dev-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "AgentOnly + UserAndAgent should match alice write on dev-agent, got: {resp:?}"
    );

    // Alice write on other-agent — AgentOnly matches but UserAndAgent doesn't → Denied
    let resp = engine.evaluate(file_write_request("other-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "AgentOnly match without UserAndAgent should be denied, got: {resp:?}"
    );
}

// ── State transition: Owner approval with --user-only ────────────────────────

#[tokio::test]
async fn test_build_whitelist_rule_user_only() {
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent".to_string(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "dev-agent".to_string(),
        path: "/data/file.txt".to_string(),
        op: "read".into(),
    };
    let rule =
        build_whitelist_rule(&caller, &body, "wl-user-only", WhitelistTarget::UserOnly).unwrap();

    assert_eq!(rule.name, "wl-user-only");
    assert_eq!(rule.effect, Effect::Allow);
    assert!(rule.subject.is_user_only());
    assert_eq!(rule.subject.user_id(), "ou_alice");
    assert_eq!(rule.subject.agent_id(), "");
}

#[tokio::test]
async fn test_user_only_whitelist_rule_effective_in_engine() {
    // Simulate: owner approves with --user-only, rule is written, then new request comes in
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("wl-user-only-001")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Alice on a completely different agent should be allowed
    let resp = engine.evaluate(
        file_read_request("completely-different-agent", "ou_alice", ""),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "UserOnly whitelist should allow alice on any agent, got: {resp:?}"
    );
}

// ── Error path: UserOnly in Owner shortcut (skipped) ─────────────────────────

#[tokio::test]
async fn test_owner_shortcut_skips_user_only_rules() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-read-deny")
                .subject(Subject::UserOnly {
                    user_id: "owner".to_string(),
                    match_type: MatchType::Exact,
                })
                .deny()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Allow)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Owner caller — should skip User phase entirely, use Agent defaults
    let resp = engine.evaluate(file_read_request("any-agent", "owner", ""), None);
    // Owner shortcut: UserOnly rules don't participate in Agent phase.
    // Agent phase has no matching rule → defaults.file_read = Allow → Allowed
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Owner shortcut should skip UserOnly rules, got: {resp:?}"
    );
}

// ── Glob matching for UserOnly ───────────────────────────────────────────────

#[tokio::test]
async fn test_user_only_glob_matching() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("team-allow-read")
                .subject(Subject::UserOnly {
                    user_id: "ou_team_*".to_string(),
                    match_type: MatchType::Glob,
                })
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // ou_team_dev should match glob
    let resp = engine.evaluate(file_read_request("any-agent", "ou_team_dev", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "UserOnly glob should match ou_team_dev, got: {resp:?}"
    );

    // ou_alice should NOT match glob
    let resp = engine.evaluate(file_read_request("any-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly glob should not match ou_alice, got: {resp:?}"
    );
}

// ── UserOnly deny + UserAndAgent allow intersection ──────────────────────────

#[tokio::test]
async fn test_user_only_deny_with_user_and_agent_allow() {
    // Alice is denied all writes via UserOnly, but allowed on specific agent via UserAndAgent.
    // UserAndAgent deny should take precedence in the user phase.
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("alice-write-deny")
                .subject(Subject::UserOnly {
                    user_id: "ou_alice".to_string(),
                    match_type: MatchType::Exact,
                })
                .deny()
                .action(Action::File {
                    operation: "write".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("alice-dev-write-allow")
                .subject(Subject::UserAndAgent {
                    user_id: "ou_alice".to_string(),
                    agent: "dev-agent".to_string(),
                    user_match: MatchType::Exact,
                    agent_match: MatchType::Exact,
                })
                .allow()
                .action(Action::File {
                    operation: "write".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_write(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Alice write on dev-agent — both rules match in user phase.
    // Deny-precedence: UserOnly deny should win.
    let resp = engine.evaluate(file_write_request("dev-agent", "ou_alice", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly deny should take precedence over UserAndAgent allow (deny wins), got: {resp:?}"
    );
}

// ── Subject accessor tests for UserOnly ──────────────────────────────────────

#[test]
fn test_subject_user_only_agent_id_returns_empty() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    assert_eq!(subject.agent_id(), "");
}

#[test]
fn test_subject_user_only_user_id_returns_user_id() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    assert_eq!(subject.user_id(), "ou_alice");
}

#[test]
fn test_subject_user_only_is_agent_only_false() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    assert!(!subject.is_agent_only());
}

#[test]
fn test_subject_user_only_is_user_only_true() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    assert!(subject.is_user_only());
}

#[test]
fn test_subject_user_only_is_user_and_agent_false() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    assert!(!subject.is_user_and_agent());
}

// ── Subject.matches() for UserOnly ───────────────────────────────────────────

#[test]
fn test_user_only_matches_exact() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "any-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_only_matches_glob() {
    let subject = Subject::UserOnly {
        user_id: "ou_team_*".to_string(),
        match_type: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_team_dev".to_string(),
        agent: "any-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_only_not_matches_different_user_exact() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_bob".to_string(),
        agent: "any-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

#[test]
fn test_user_only_not_matches_different_user_glob() {
    let subject = Subject::UserOnly {
        user_id: "ou_team_*".to_string(),
        match_type: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "any-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

// ── Serde round-trip for UserOnly Subject ────────────────────────────────────

#[test]
fn test_user_only_subject_serde_round_trip() {
    let subject = Subject::UserOnly {
        user_id: "ou_alice".to_string(),
        match_type: MatchType::Exact,
    };
    let json = serde_json::to_string(&subject).unwrap();
    assert!(json.contains("\"user_only\""));
    let deserialized: Subject = serde_json::from_str(&json).unwrap();
    assert!(deserialized.is_user_only());
    assert_eq!(deserialized.user_id(), "ou_alice");
}
