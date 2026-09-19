//! Caller-semantics tests for `PermissionRequestBody::ToolCall` (plan
//! Step 1.6, around the Step 1.1 tool-dispatch caller wiring).
//!
//! The gateway tool dispatch builds Level-1 `ToolCall` requests with
//! `Caller { user_id, agent }` where `user_id` comes from the session
//! checkpoint sender (`resolve_caller_user_id`, design "User 来源").
//! These tests construct `Caller` directly and pin the plan behavior
//! dimensions on the `ToolCall` request body (the existing owner /
//! two-phase suites pin them on `FileOp`):
//!
//! - ① normal path: `user_id="owner"` → User dimension exempt, only
//!   the Agent dimension decides;
//! - ② intersection: non-owner two-phase merge — Agent Allow + User
//!   Deny → Deny, Agent Allow + User Allow → Allow;
//! - ④ error/boundary: empty `user_id` (dispatch fallback when there
//!   is no checkpoint / no sender / no session id) → engine empty-uid
//!   branch semantics (User phase skipped; falls back to Agent
//!   defaults, unlike an identified non-owner who hits user_defaults).
//!
//! Dimension ③ (dispatch-side source of `user_id`) is covered by the
//! gateway-side tests in `crates/gateway/src/session_handler_tool_dispatch.rs`.

use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Action, Caller, Defaults, Effect, MatchType, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Rule, RuleSet, Subject,
};

const AGENT: &str = "master";
const SKILL: &str = "file_ops";
const METHOD: &str = "Read";

/// Agent-dimension defaults (design default: everything Deny; `message`
/// is not restricted by default). `tool_call` is parameterized so the
/// empty-uid fallback test can tell Agent defaults from user_defaults.
fn agent_defaults(tool_call: Effect) -> Defaults {
    Defaults {
        file_read: Effect::Deny,
        file_write: Effect::Deny,
        exec: Effect::Deny,
        network: Effect::Deny,
        inter_agent: Effect::Deny,
        config: Effect::Deny,
        tool_call,
        message: Effect::Allow,
    }
}

fn make_engine(defaults: Defaults, rules: Vec<Rule>) -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSet {
        rules,
        defaults,
        user_defaults: Defaults::user_defaults(),
        template_includes: vec![],
        rule_version: String::new(),
    })
}

/// Level-1 ToolCall request body, as built by the tool dispatch.
fn tool_call_request(user_id: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: AGENT.to_string(),
        },
        request: PermissionRequestBody::ToolCall {
            agent: AGENT.to_string(),
            skill: SKILL.to_string(),
            method: METHOD.to_string(),
        },
    }
}

fn rule(name: &str, subject: Subject, effect: Effect) -> Rule {
    Rule {
        name: name.to_string(),
        subject,
        effect,
        actions: vec![Action::ToolCall {
            skill: SKILL.to_string(),
            methods: vec![METHOD.to_string()],
        }],
        template: None,
        priority: 10,
    }
}

fn agent_rule(name: &str, effect: Effect) -> Rule {
    rule(
        name,
        Subject::AgentOnly {
            agent: AGENT.to_string(),
            match_type: MatchType::Exact,
        },
        effect,
    )
}

fn user_rule(name: &str, user_id: &str, effect: Effect) -> Rule {
    rule(
        name,
        Subject::UserAndAgent {
            user_id: user_id.to_string(),
            agent: AGENT.to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        },
        effect,
    )
}

// -------------------------------------------------------------------------
// ① Normal path: owner → User dimension exempt, Agent dimension decides
// -------------------------------------------------------------------------

/// Owner + Agent Allow + User Deny → Allowed: the User-dimension rule is
/// exempted at the engine Owner shortcut, so it cannot veto the call.
#[test]
fn test_owner_user_deny_exempted_agent_allow_allowed() {
    let engine = make_engine(
        agent_defaults(Effect::Deny),
        vec![
            agent_rule("agent-allow-read", Effect::Allow),
            user_rule("user-deny-read", "owner", Effect::Deny),
        ],
    );
    let resp = engine.evaluate(tool_call_request("owner"), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "owner must be exempt from the User-dimension deny (only Agent \
         dimension decides), got {resp:?}"
    );
}

/// Owner + Agent Deny + User Allow → Denied (rule `agent-deny-read`):
/// the User dimension cannot rescue a call the Agent dimension denies.
#[test]
fn test_owner_agent_deny_user_allow_denied() {
    let engine = make_engine(
        agent_defaults(Effect::Deny),
        vec![
            agent_rule("agent-deny-read", Effect::Deny),
            user_rule("user-allow-read", "owner", Effect::Allow),
        ],
    );
    let resp = engine.evaluate(tool_call_request("owner"), None);
    assert!(
        matches!(
            resp,
            PermissionResponse::Denied { ref rule, .. } if rule == "agent-deny-read"
        ),
        "owner: only the Agent dimension decides, got {resp:?}"
    );
}

// -------------------------------------------------------------------------
// ② Intersection path: non-owner two-phase merge
// -------------------------------------------------------------------------

/// Non-owner + Agent Allow + User Deny → Denied (`<user_phase>` merge):
/// a User-dimension Deny vetoes the intersection.
#[test]
fn test_non_owner_agent_allow_user_deny_denied() {
    let engine = make_engine(
        agent_defaults(Effect::Deny),
        vec![
            agent_rule("agent-allow-read", Effect::Allow),
            user_rule("user-deny-read", "alice", Effect::Deny),
        ],
    );
    let resp = engine.evaluate(tool_call_request("alice"), None);
    assert!(
        matches!(
            resp,
            PermissionResponse::Denied { ref rule, .. } if rule == "<user_phase>"
        ),
        "Agent Allow + User Deny must intersect to Deny, got {resp:?}"
    );
}

/// Non-owner + Agent Allow + User Allow → Allowed: both dimensions must
/// Allow (design intersection model).
#[test]
fn test_non_owner_agent_allow_user_allow_allowed() {
    let engine = make_engine(
        agent_defaults(Effect::Deny),
        vec![
            agent_rule("agent-allow-read", Effect::Allow),
            user_rule("user-allow-read", "alice", Effect::Allow),
        ],
    );
    let resp = engine.evaluate(tool_call_request("alice"), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Agent Allow + User Allow must intersect to Allow, got {resp:?}"
    );
}

// -------------------------------------------------------------------------
// ④ Error/boundary: empty user_id (dispatch fallback) → engine empty-uid branch
// -------------------------------------------------------------------------

/// Empty user_id + Agent Allow → Allowed: the dispatch fallback (no
/// checkpoint / no sender / no session id) resolves to an empty
/// user_id, and the engine skips the User phase for it.
#[test]
fn test_empty_user_id_agent_allow_allowed() {
    let engine = make_engine(
        agent_defaults(Effect::Deny),
        vec![agent_rule("agent-allow-read", Effect::Allow)],
    );
    let resp = engine.evaluate(tool_call_request(""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "empty user_id + Agent Allow should skip the User phase, got {resp:?}"
    );
}

/// Empty user_id with no matching rules falls back to the **Agent**
/// defaults, while an identified non-owner falls back to user_defaults
/// (all Deny) — the two fallbacks must not be conflated. Engine defaults
/// have `tool_call: Allow` here so the difference is observable.
#[test]
fn test_empty_user_id_uses_agent_defaults_unlike_identified_user() {
    let engine = make_engine(agent_defaults(Effect::Allow), vec![]);
    let empty_resp = engine.evaluate(tool_call_request(""), None);
    assert!(
        matches!(empty_resp, PermissionResponse::Allowed { .. }),
        "empty user_id should use Agent defaults (tool_call: Allow), got {empty_resp:?}"
    );
    let alice_resp = engine.evaluate(tool_call_request("alice"), None);
    assert!(
        matches!(alice_resp, PermissionResponse::Denied { .. }),
        "identified non-owner with no matching User rule must hit \
         user_defaults (all Deny), got {alice_resp:?}"
    );
}
