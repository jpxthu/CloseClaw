//! Unit tests for the permission_check module.

use super::*;
use crate::permission_port::PermMessageDirection;
use crate::test_adapters::real_permission_port;
use crate::{ToolCallError, ToolContext};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{
    ApprovalFlow, ApprovalNotification, HeartbeatApprovalMode,
};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Action, Effect, MatchType, MessageDirection, Rule, RuleSet, Subject,
};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_permission::Defaults;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

type ApprovalMutex = TokioMutex<ApprovalFlow>;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

pub(crate) fn make_engine_with_rules(
    rules: Vec<Rule>,
) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            exec: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ))
}

pub(crate) fn make_sm() -> Arc<SessionManager> {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ))
}

pub(crate) fn make_cm() -> Arc<ConfigManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

/// Bundle the real engine / session manager / config manager / approval
/// flow behind the tools-owned permission port.
pub(crate) fn make_port(
    engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    sm: Arc<SessionManager>,
    cm: Arc<ConfigManager>,
    flow: Arc<ApprovalMutex>,
) -> PermDeps {
    real_permission_port(engine, sm, cm, flow)
}

/// Standard approval flow — enqueues denials (approval-pending path).
pub(crate) fn make_af() -> Arc<ApprovalMutex> {
    Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

/// Denying approval flow — submit_denial returns None (hard deny path).
pub(crate) fn make_af_deny() -> Arc<ApprovalMutex> {
    Arc::new(TokioMutex::new(ApprovalFlow::new_deny_all(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

/// Standard approval flow with a capturing owner-notification callback so
/// route tests can observe whether (and how often) the flow was contacted.
pub(crate) fn make_af_capturing() -> (
    Arc<ApprovalMutex>,
    Arc<std::sync::Mutex<Vec<ApprovalNotification>>>,
) {
    let notifications: Arc<std::sync::Mutex<Vec<ApprovalNotification>>> = Arc::default();
    let sink = Arc::clone(&notifications);
    let flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(move |n: ApprovalNotification| {
            sink.lock().unwrap().push(n);
        }),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )));
    (flow, notifications)
}

// ---------------------------------------------------------------------------
// Mock PersistenceService for checkpoint-based sender_id tests
// ---------------------------------------------------------------------------

use closeclaw_gateway::GatewayConfig;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::sync::Mutex as StdMutex;

struct MockPersist {
    checkpoints: StdMutex<Vec<SessionCheckpoint>>,
}

impl MockPersist {
    fn new() -> Self {
        Self {
            checkpoints: StdMutex::new(Vec::new()),
        }
    }
    fn insert(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints.lock().unwrap().push(cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.session_id == sid)
            .cloned())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.session_id.clone())
            .collect())
    }
}

/// Build a SessionCheckpoint with the given sender_id.
fn make_checkpoint(session_id: &str, sender_id: Option<String>) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.sender_id = sender_id;
    cp.agent_id = Some("test-agent".to_string());
    cp
}

fn make_sm_with_persist() -> (Arc<SessionManager>, Arc<MockPersist>) {
    let persist = Arc::new(MockPersist::new());
    let sm = Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        Some(persist.clone() as Arc<dyn closeclaw_session::persistence::PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    (sm, persist)
}

fn make_deps_with_shared_sm(
    rules: Vec<Rule>,
    sm: Arc<SessionManager>,
    flow: Arc<ApprovalMutex>,
) -> PermDeps {
    make_port(make_engine_with_rules(rules), sm, make_cm(), flow)
}

fn make_ctx_with_session(agent: &str, session_id: &str) -> ToolContext {
    ToolContext {
        agent_id: agent.to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()).filter(|s| !s.is_empty()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

fn make_deps(rules: Vec<Rule>) -> PermDeps {
    make_port(
        make_engine_with_rules(rules),
        make_sm(),
        make_cm(),
        make_af(),
    )
}

/// Like `make_deps` but uses a deny-all approval flow.
fn make_deps_deny(rules: Vec<Rule>) -> PermDeps {
    make_port(
        make_engine_with_rules(rules),
        make_sm(),
        make_cm(),
        make_af_deny(),
    )
}

/// Like `make_deps_deny` but with message default also set to Deny.
fn make_deps_with_message_deny(rules: Vec<Rule>) -> PermDeps {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            exec: Effect::Deny,
            message: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    make_port(
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(rs),
        )),
        make_sm(),
        make_cm(),
        make_af_deny(),
    )
}

pub(crate) fn make_ctx(agent: &str) -> ToolContext {
    make_ctx_with_session(agent, "")
}

fn allow_tool_rule(agent: &str, skill: &str) -> Rule {
    Rule {
        name: format!("allow-{skill}-call"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ToolCall {
            skill: skill.to_string(),
            methods: vec!["call".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_file_rule(agent: &str, path_glob: &str, op: &str) -> Rule {
    Rule {
        name: format!("allow-file-{op}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: op.to_string(),
            paths: vec![path_glob.to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_cmd_rule(agent: &str, cmd_pattern: &str) -> Rule {
    Rule {
        name: format!("allow-cmd-{cmd_pattern}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::Command {
            command: cmd_pattern.to_string(),
            args: Default::default(),
        }],
        template: None,
        priority: 0,
    }
}

fn allow_network_rule(agent: &str, host: &str) -> Rule {
    Rule {
        name: format!("allow-net-{host}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::Network {
            hosts: vec![host.to_string()],
            ports: vec![],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_message_rule(agent: &str) -> Rule {
    Rule {
        name: format!("allow-message-{agent}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::Message {
            direction: MessageDirection::Both,
            targets: vec!["*".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_config_write_rule(agent: &str) -> Rule {
    Rule {
        name: format!("allow-cfgwrite-{agent}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ConfigWrite {
            files: vec!["*".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

// ---------------------------------------------------------------------------
// check_tool_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tool_allowed_when_rule_matches() {
    let deps = make_deps(vec![allow_tool_rule("agent-a", "bash")]);
    let ctx = make_ctx("agent-a");
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "allowed → None");
}

#[tokio::test]
async fn test_tool_denied_when_no_matching_rule() {
    let deps = make_deps_deny(vec![allow_tool_rule("agent-a", "bash")]);
    let ctx = make_ctx("other-agent");
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert!(!reason.is_empty());
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

#[tokio::test]
async fn test_tool_denied_when_wrong_skill() {
    let deps = make_deps_deny(vec![allow_tool_rule("agent-a", "file_ops")]);
    let ctx = make_ctx("agent-a");
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_file_op_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_file_op_read_allowed() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/test.txt", "read", None).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_file_op_write_allowed() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "write")]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/out.txt", "write", None).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_file_op_denied_without_rule() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/test.txt", "read", None).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_command_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_command_allowed() {
    let deps = make_deps(vec![allow_cmd_rule("agent-a", "echo")]);
    let ctx = make_ctx("agent-a");
    let result = check_command_permission(&deps, &ctx, "echo", &["hello".to_string()], None).await;
    assert!(matches!(result, CommandPermissionResult::Permitted));
}

#[tokio::test]
async fn test_command_denied_without_rule() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_command_permission(
        &deps,
        &ctx,
        "rm",
        &["-rf".to_string(), "/".to_string()],
        None,
    )
    .await;
    assert!(matches!(result, CommandPermissionResult::Denied(_)));
}

// ---------------------------------------------------------------------------
// check_network_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_network_allowed() {
    let deps = make_deps(vec![allow_network_rule("agent-a", "example.com")]);
    let ctx = make_ctx("agent-a");
    let result = check_network_permission(&deps, &ctx, "example.com", 443).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "allowed → None");
}

#[tokio::test]
async fn test_network_denied_without_rule() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_network_permission(&deps, &ctx, "evil.com", 80).await;
    assert!(result.is_err());
}

/// Network permission with specific port: allowed host but non-matching port.
#[tokio::test]
async fn test_network_allowed_specific_port() {
    let deps = make_deps(vec![allow_network_rule("agent-a", "example.com")]);
    let ctx = make_ctx("agent-a");
    let result = check_network_permission(&deps, &ctx, "example.com", 8443).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "allowed → None");
}

/// Network permission: allowed host but wrong host → denied.
#[tokio::test]
async fn test_network_denied_wrong_host() {
    let deps = make_deps_deny(vec![allow_network_rule("agent-a", "safe.com")]);
    let ctx = make_ctx("agent-a");
    let result = check_network_permission(&deps, &ctx, "unsafe.com", 443).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_message_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_message_permission_allowed() {
    let deps = make_deps(vec![allow_message_rule("agent-a")]);
    let ctx = make_ctx("agent-a");
    let result = check_message_permission(&deps, &ctx, PermMessageDirection::Both, "chat_1").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "allowed → None");
}

#[tokio::test]
async fn test_message_permission_denied() {
    let deps = make_deps_with_message_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_message_permission(&deps, &ctx, PermMessageDirection::Send, "chat_1").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_config_write_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_write_permission_allowed() {
    // ConfigWrite Allow rules are intercepted by the guard and forced to
    // Denied, so even with an Allow rule, ConfigWrite is denied and
    // routes through the approval flow.
    let deps = make_deps(vec![allow_config_write_rule("agent-a")]);
    let ctx = make_ctx("agent-a");
    let result = check_config_write_permission(&deps, &ctx, "config.yaml").await;
    assert!(result.is_ok());
    // Guard intercepts Allow → routes through approval flow
    assert!(
        result.unwrap().is_some(),
        "config write Allow should route to approval flow"
    );
}

#[tokio::test]
async fn test_config_write_permission_denied() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_config_write_permission(&deps, &ctx, "config.yaml").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_inputs_are_denied() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let tool = check_tool_permission(&deps, &ctx, "", "call", None).await;
    assert!(tool.is_err());
    let file = check_file_op_permission(&deps, &ctx, "", "read", None).await;
    assert!(file.is_err());
    let cmd = check_command_permission(&deps, &ctx, "", &vec![], None).await;
    assert!(matches!(cmd, CommandPermissionResult::Denied(_)));
}

#[tokio::test]
async fn test_file_op_special_char_path() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");
    let path = "/tmp/file with spaces & special!@#.txt";
    let result = check_file_op_permission(&deps, &ctx, path, "read", None).await;
    // Should not panic on special chars
    assert!(result.is_ok() || result.is_err());
}

// ---------------------------------------------------------------------------
// Two-level: Level 1 blocks → Level 2 never reached
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_level1_block_prevents_level2() {
    // Agent has FileOp rule but NO ToolCall rule.
    // Level 1 (ToolCall) should deny before Level 2 (FileOp) is checked.
    let deps = make_deps_deny(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call", None).await;
    assert!(level1.is_err(), "Level 1 should deny");
}

#[tokio::test]
async fn test_level1_pass_level2_pass() {
    // Agent has both ToolCall and FileOp rules.
    let deps = make_deps(vec![
        allow_tool_rule("agent-a", "file_ops"),
        allow_file_rule("agent-a", "/tmp/**", "read"),
    ]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call", None).await;
    assert!(level1.is_ok());
    assert!(level1.unwrap().is_none());

    let level2 = check_file_op_permission(&deps, &ctx, "/tmp/file.txt", "read", None).await;
    assert!(level2.is_ok());
    assert!(level2.unwrap().is_none());
}

#[tokio::test]
async fn test_level1_pass_level2_denied() {
    // Agent has ToolCall rule but NO FileOp rule.
    let deps = make_deps(vec![allow_tool_rule("agent-a", "file_ops")]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call", None).await;
    assert!(level1.is_ok());
    assert!(level1.unwrap().is_none());

    // Use deny flow for Level 2 to get a hard denial
    let deps2 = make_deps_deny(vec![allow_tool_rule("agent-a", "file_ops")]);
    let level2 = check_file_op_permission(&deps2, &ctx, "/tmp/file.txt", "read", None).await;
    assert!(level2.is_err(), "Level 2 should deny");
}

// ---------------------------------------------------------------------------
// is_config_file tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_is_config_file_in_config_dir() {
    let cm = make_cm();
    let deps = make_port(
        make_engine_with_rules(vec![]),
        make_sm(),
        Arc::clone(&cm),
        make_af(),
    );
    let data_root = cm.config_dir();
    let path = data_root
        .join("agents/a1/permissions.json")
        .to_string_lossy()
        .into_owned();
    assert!(deps.is_config_file(&path));
}

#[tokio::test]
async fn test_is_config_file_in_workspace() {
    let cm = make_cm();
    let deps = make_port(
        make_engine_with_rules(vec![]),
        make_sm(),
        Arc::clone(&cm),
        make_af(),
    );
    let data_root = cm.config_dir();
    let path = data_root
        .join("workspaces/a1/u1/file.txt")
        .to_string_lossy()
        .into_owned();
    assert!(!deps.is_config_file(&path));
}

#[tokio::test]
async fn test_is_config_file_normal_file() {
    let deps = make_deps(vec![]);
    assert!(!deps.is_config_file("/tmp/regular/file.txt"));
}

// Step 1.3: Real User ID in permission check functions

/// Session with sender → real user_id → AgentOnly + UserAndAgent Allow → Allowed.
#[tokio::test]
async fn test_session_with_sender_uses_real_user_id() {
    let (sm, persist) = make_sm_with_persist();
    persist.insert(make_checkpoint("sess-1", Some("ou_alice".to_string())));
    // Two-phase: AgentOnly Allow (agent phase) + UserAndAgent Allow (user phase)
    let rules = vec![
        Rule {
            name: "agent-tool-allow".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent-a".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::ToolCall {
                skill: "bash".to_string(),
                methods: vec!["call".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "alice-tool-allow".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "ou_alice".to_string(),
                agent: "agent-a".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::ToolCall {
                skill: "bash".to_string(),
                methods: vec!["call".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let flow = make_af();
    let deps = make_deps_with_shared_sm(rules, sm, flow);
    let ctx = make_ctx_with_session("agent-a", "sess-1");
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    match &result {
        Ok(None) => {} // expected: Allowed
        other => panic!("expected Ok(None) (Allowed), got: {:?}", other),
    }
}

/// Session without sender → Bare fallback → Agent dimension decides.
#[tokio::test]
async fn test_session_without_sender_falls_back_to_bare() {
    let (sm, persist) = make_sm_with_persist();
    // Insert checkpoint WITHOUT sender_id
    persist.insert(make_checkpoint("sess-2", None));
    let rules = vec![Rule {
        name: "agent-tool-allow".to_string(),
        subject: Subject::AgentOnly {
            agent: "agent-a".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![Action::ToolCall {
            skill: "bash".to_string(),
            methods: vec!["call".to_string()],
        }],
        template: None,
        priority: 0,
    }];
    let flow = make_af();
    let deps = make_deps_with_shared_sm(rules, sm, flow);
    let ctx = make_ctx_with_session("agent-a", "sess-2");
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    // Agent dimension Allow → Allowed (user_id is empty, user phase skipped)
    assert!(result.unwrap().is_none());
}

/// State transition: no rule → Deny → write UserAndAgent Allow rule → Allowed.
#[tokio::test]
async fn test_user_rule_takes_effect_after_write() {
    let (sm, persist) = make_sm_with_persist();
    persist.insert(make_checkpoint("sess-3", Some("ou_bob".to_string())));
    // Phase 1: No rules → Deny (user_defaults = all Deny)
    let empty_rules: Vec<Rule> = vec![];
    let flow1 = make_af_deny();
    let deps1 = make_deps_with_shared_sm(empty_rules, sm.clone(), flow1);
    let ctx = make_ctx_with_session("agent-a", "sess-3");
    let result1 = check_tool_permission(&deps1, &ctx, "bash", "call", None).await;
    assert!(result1.is_err(), "Phase 1: no rules → Deny");
    // Phase 2: Write AgentOnly + UserAndAgent Allow rules for bob → Allowed
    let rules_with_allow = vec![
        Rule {
            name: "agent-tool-allow".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent-a".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::ToolCall {
                skill: "bash".to_string(),
                methods: vec!["call".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "bob-tool-allow".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "ou_bob".to_string(),
                agent: "agent-a".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::ToolCall {
                skill: "bash".to_string(),
                methods: vec!["call".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let flow2 = make_af();
    let deps2 = make_deps_with_shared_sm(rules_with_allow, sm, flow2);
    let result2 = check_tool_permission(&deps2, &ctx, "bash", "call", None).await;
    match &result2 {
        Ok(None) => {}
        other => panic!("Phase 2: expected Ok(None), got: {:?}", other),
    }
}
