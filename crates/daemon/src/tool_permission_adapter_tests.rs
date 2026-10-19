//! Unit tests for [`crate::tool_permission_adapter`] — the daemon
//! tools-side permission check adapter.
//!
//! Split from `tool_permission_adapter.rs` (STANDARDS.md §2: unit tests
//! live in a separate `<module>_tests.rs`, not inline `#[cfg(test)]`).
//! Mounted via `#[path]` (trait_adapters.rs / llm_init.rs precedent).

use super::*;
use closeclaw_permission::Effect;
use closeclaw_session::persistence::{PersistenceError, SessionCheckpoint};
use std::collections::HashMap;

// ── Enum mapping（正常路径：全变体覆盖） ─────────────────────────

#[test]
fn message_direction_mapping_covers_all_variants() {
    for (permission_dir, perm_dir) in [
        (MessageDirection::Send, PermMessageDirection::Send),
        (MessageDirection::Receive, PermMessageDirection::Receive),
        (MessageDirection::Both, PermMessageDirection::Both),
    ] {
        assert_eq!(message_direction_to_permission(&perm_dir), permission_dir);
    }
}

// ── Body mapping（字段逐项保留） ─────────────────────────────────

#[test]
fn body_mapping_preserves_all_fields_for_every_variant() {
    let cases: Vec<(PermRequestBody, PermissionRequestBody)> = vec![
        (
            PermRequestBody::ToolCall {
                agent: "a".into(),
                skill: "bash".into(),
                method: "call".into(),
            },
            PermissionRequestBody::ToolCall {
                agent: "a".into(),
                skill: "bash".into(),
                method: "call".into(),
            },
        ),
        (
            PermRequestBody::FileOp {
                agent: "a".into(),
                path: "/tmp/x".into(),
                op: "read".into(),
            },
            PermissionRequestBody::FileOp {
                agent: "a".into(),
                path: "/tmp/x".into(),
                op: "read".into(),
            },
        ),
        (
            PermRequestBody::MessageSend {
                agent: "a".into(),
                direction: PermMessageDirection::Send,
                target: "chat_1".into(),
            },
            PermissionRequestBody::MessageSend {
                agent: "a".into(),
                direction: MessageDirection::Send,
                target: "chat_1".into(),
            },
        ),
        (
            PermRequestBody::ConfigWrite {
                agent: "a".into(),
                config_file: "config.yaml".into(),
            },
            PermissionRequestBody::ConfigWrite {
                agent: "a".into(),
                config_file: "config.yaml".into(),
            },
        ),
        (
            PermRequestBody::NetOp {
                agent: "a".into(),
                host: "example.com".into(),
                port: 443,
            },
            PermissionRequestBody::NetOp {
                agent: "a".into(),
                host: "example.com".into(),
                port: 443,
            },
        ),
        (
            PermRequestBody::CommandExec {
                agent: "a".into(),
                cmd: "echo".into(),
                args: vec!["hi".into()],
            },
            PermissionRequestBody::CommandExec {
                agent: "a".into(),
                cmd: "echo".into(),
                args: vec!["hi".into()],
            },
        ),
    ];
    for (tools_body, expected) in cases {
        // Field preservation is observable through the permission-side
        // dimension + agent accessors plus serde round-trip equality.
        let mapped = body_to_permission(&tools_body);
        assert_eq!(
            serde_json::to_value(&mapped).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(mapped.dimension_name(), expected.dimension_name());
    }
}

// ── Verdict mapping（错误/边界：Allowed、Denied 各形态） ─────────

#[test]
fn verdict_mapping_preserves_denied_fields_and_drops_unused() {
    let verdict = verdict_from_permission(PermissionResponse::Allowed {
        token: "tok".into(),
        context_modifier: Some("modifier".into()),
    });
    assert_eq!(verdict, PermVerdict::Allowed);

    let verdict = verdict_from_permission(PermissionResponse::Denied {
        reason: "no rule".into(),
        rule: "r1".into(),
        risk_level: RiskLevel::High,
        approval_request_id: None,
    });
    assert_eq!(
        verdict,
        PermVerdict::Denied {
            reason: "no rule".into(),
            risk_level: RiskLevel::High,
            approval_request_id: None,
        }
    );

    let verdict = verdict_from_permission(PermissionResponse::Denied {
        reason: "already queued".into(),
        rule: "r2".into(),
        risk_level: RiskLevel::Critical,
        approval_request_id: Some("req-1".into()),
    });
    assert_eq!(
        verdict,
        PermVerdict::Denied {
            reason: "already queued".into(),
            risk_level: RiskLevel::Critical,
            approval_request_id: Some("req-1".into()),
        }
    );
}

// ── Config-file classification against a real ConfigManager ──────

#[tokio::test]
async fn is_config_file_classifies_config_tree_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_manager = Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap());
    let session_manager = test_session_manager();
    let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
        tmp.path().to_path_buf(),
        closeclaw_permission::RuleSet::default(),
    )));
    let adapter = ToolPermissionAdapter::new(
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(closeclaw_permission::RuleSet::default()),
        )),
        session_manager,
        config_manager,
        approval_flow,
    );
    let data_root = tmp.path().to_path_buf();
    let inside = data_root
        .join("agents/a1/permissions.json")
        .to_string_lossy()
        .into_owned();
    let outside = tmp
        .path()
        .join("workspaces/a1/file.txt")
        .to_string_lossy()
        .into_owned();
    assert!(adapter.is_config_file(&inside));
    assert!(!adapter.is_config_file(&outside));
}

// ── Sub-agent decision（空/不存在会话的安全默认） ────────────────

#[tokio::test]
async fn is_session_sub_agent_defaults_false_for_empty_or_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let session_manager = test_session_manager();
    let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
        tmp.path().to_path_buf(),
        closeclaw_permission::RuleSet::default(),
    )));
    let adapter = ToolPermissionAdapter::new(
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(closeclaw_permission::RuleSet::default()),
        )),
        Arc::clone(&session_manager),
        Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap()),
        approval_flow,
    );
    assert!(!adapter.is_session_sub_agent("").await);
    assert!(!adapter.is_session_sub_agent("does-not-exist").await);
}

#[tokio::test]
async fn factory_returns_trait_object() {
    let tmp = tempfile::TempDir::new().unwrap();
    let session_manager = test_session_manager();
    let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
        tmp.path().to_path_buf(),
        closeclaw_permission::RuleSet::default(),
    )));
    let port: Arc<dyn ToolPermissionCheck> = tool_permission_check(
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(closeclaw_permission::RuleSet::default()),
        )),
        session_manager,
        Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap()),
        approval_flow,
    );
    assert!(!port.is_config_file("/tmp/regular/file.txt"));
}

// ── evaluate：checkpoint Owner 注入路径 ─────────────────────────

/// Minimal in-memory `PersistenceService` double: gateway's
/// `SnapshotMockPersist` is `pub(crate)` and not exported, so tests
/// here keep a local HashMap-backed stub — just enough for
/// `SessionManager::get_sender_id` to resolve a bound checkpoint
/// sender (no real network / LLM, no real storage).
struct MockPersist {
    checkpoints: TokioMutex<HashMap<String, SessionCheckpoint>>,
}

impl MockPersist {
    fn new() -> Self {
        Self {
            checkpoints: TokioMutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl closeclaw_session::persistence::PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(sid).cloned())
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

/// SessionManager backed by in-memory mock persistence; when
/// `sender` is `Some`, the checkpoint for `session_id` binds it as
/// the session Owner (the value `get_sender_id` resolves).
async fn session_manager_with_checkpoint_sender(
    session_id: &str,
    sender: Option<&str>,
) -> Arc<SessionManager> {
    let persist =
        Arc::new(MockPersist::new()) as Arc<dyn closeclaw_session::persistence::PersistenceService>;
    let mut cp = SessionCheckpoint::new(session_id.to_owned());
    cp.sender_id = sender.map(str::to_owned);
    persist.save_checkpoint(&cp).await.unwrap();
    Arc::new(SessionManager::new(
        &closeclaw_gateway::GatewayConfig::default(),
        Some(persist),
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ))
}

/// All-deny defaults (agent + user), no rules: only a hardcoded
/// path can produce `Allowed`, so any verdict is attributable.
fn all_deny_ruleset() -> closeclaw_permission::RuleSet {
    closeclaw_permission::RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap()
}

/// Ruleset isolating the Owner shortcut as the only path to
/// `Allowed` for a file read: user_defaults are all-Deny, an
/// AgentOnly rule allows reads, and a UserOnly deny rule targets
/// the session Owner id — a non-owner caller with the resolved id
/// is denied by the User phase, so `Allowed` ⇔ Owner shortcut.
fn owner_shortcut_ruleset() -> closeclaw_permission::RuleSet {
    closeclaw_permission::RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .rule(
            closeclaw_permission::RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    closeclaw_permission::actions::ActionBuilder::file(
                        "read",
                        vec!["/data/**".to_string()],
                    )
                    .build()
                    .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            closeclaw_permission::RuleBuilder::new()
                .name("user-only-deny-read")
                .subject(closeclaw_permission::Subject::UserOnly {
                    user_id: "ou_real_owner".to_string(),
                    match_type: closeclaw_permission::MatchType::Exact,
                })
                .deny()
                .action(
                    closeclaw_permission::actions::ActionBuilder::file(
                        "read",
                        vec!["**".to_string()],
                    )
                    .build()
                    .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap()
}

/// Ruleset discriminating the Owner shortcut from the empty-user
/// system-caller path for a file read: an AgentOnly allow alone would
/// let both through (an empty `caller.user_id` merges as a system
/// caller even without the shortcut), so a
/// `UserOnly { user_id: "" }` deny rule marks the difference — it is
/// consulted only when the User phase runs, i.e. only when the Owner
/// shortcut did NOT fire. User-phase deny ⇔ no Owner shortcut;
/// `Allowed` ⇔ an empty-string sender wrongly fired the shortcut.
fn empty_sender_discriminator_ruleset() -> closeclaw_permission::RuleSet {
    closeclaw_permission::RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .rule(
            closeclaw_permission::RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    closeclaw_permission::actions::ActionBuilder::file(
                        "read",
                        vec!["/data/**".to_string()],
                    )
                    .build()
                    .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            closeclaw_permission::RuleBuilder::new()
                .name("empty-user-deny-read")
                .subject(closeclaw_permission::Subject::UserOnly {
                    user_id: String::new(),
                    match_type: closeclaw_permission::MatchType::Exact,
                })
                .deny()
                .action(
                    closeclaw_permission::actions::ActionBuilder::file(
                        "read",
                        vec!["**".to_string()],
                    )
                    .build()
                    .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap()
}

/// Adapter over an engine with the given ruleset; approval flow and
/// config manager point at `config_dir` (caller-owned TempDir).
fn adapter_with_ruleset(
    config_dir: &std::path::Path,
    ruleset: closeclaw_permission::RuleSet,
    session_manager: Arc<SessionManager>,
) -> ToolPermissionAdapter {
    let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
        config_dir.to_path_buf(),
        closeclaw_permission::RuleSet::default(),
    )));
    ToolPermissionAdapter::new(
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(ruleset),
        )),
        session_manager,
        Arc::new(ConfigManager::new(config_dir.to_path_buf()).unwrap()),
        approval_flow,
    )
}

/// get_sender_id 返回 Some：解析出的 checkpoint sender 写回 caller
/// （Bare→WithCaller 升级可观测），并以同值注入 owner_id → 引擎
/// Owner 短路生效：user_defaults Deny 与 UserOnly deny 均被跳过，
/// 仅由 AgentOnly allow 放行（无注入的同身份 caller 必被 UserOnly
/// deny 拒绝，故 Allowed 唯一对应 Owner 短路）。
#[tokio::test]
async fn evaluate_resolves_sender_and_applies_owner_shortcut() {
    let tmp = tempfile::TempDir::new().unwrap();
    let session_manager =
        session_manager_with_checkpoint_sender("s-owner", Some("ou_real_owner")).await;
    let adapter = adapter_with_ruleset(
        tmp.path(),
        owner_shortcut_ruleset(),
        Arc::clone(&session_manager),
    );
    let mut caller = PermCaller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
    };
    let verdict = adapter
        .evaluate(
            Some("s-owner"),
            &mut caller,
            &PermRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/data/file.txt".to_string(),
                op: "read".to_string(),
            },
        )
        .await;
    assert_eq!(
        verdict,
        PermVerdict::Allowed,
        "session Owner must pass via the Owner shortcut: {:?}",
        verdict
    );
    assert_eq!(
        caller.user_id, "ou_real_owner",
        "resolved checkpoint sender must be written back to the caller (Bare→WithCaller upgrade)"
    );
}

/// checkpoint sender 为空串（`Some("")`）：与 gateway
/// `session_handler_tool_dispatch` 注入语义一致，空串归一为「无
/// sender」→ 请求保持 Bare、不注入 owner_id → Owner 短路不得触发。
/// 判别式（见 `empty_sender_discriminator_ruleset`）：若回归为
/// `Some("")` 注入，`""==""` 触发短路跳过 User 阶段 → AgentOnly
/// allow 放行 → Allowed；正确行为下 UserOnly("") deny 命中 →
/// user-phase 拒绝（等效可观测断言）。
#[tokio::test]
async fn evaluate_with_empty_checkpoint_sender_does_not_fire_owner_shortcut() {
    let tmp = tempfile::TempDir::new().unwrap();
    let session_manager = session_manager_with_checkpoint_sender("s-empty", Some("")).await;
    let adapter = adapter_with_ruleset(
        tmp.path(),
        empty_sender_discriminator_ruleset(),
        session_manager,
    );
    let mut caller = PermCaller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
    };
    let verdict = adapter
        .evaluate(
            Some("s-empty"),
            &mut caller,
            &PermRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/data/file.txt".to_string(),
                op: "read".to_string(),
            },
        )
        .await;
    assert!(
        matches!(
            verdict,
            PermVerdict::Denied {
                ref reason,
                approval_request_id: None,
                ..
            } if reason == "action denied by user rule"
        ),
        "empty checkpoint sender must not fire the Owner shortcut (UserOnly(\"\") deny must apply): {:?}",
        verdict
    );
    assert_eq!(
        caller.user_id, "",
        "empty checkpoint sender must not be written back to the caller"
    );
}

/// get_sender_id 返回 None（checkpoint 存在但未绑定 sender）：请求
/// 保持 Bare（caller 不被改写、无 owner_id 注入）→ 无 Owner 短路，
/// 走 defaults（全 Deny）→ 拒绝。
#[tokio::test]
async fn evaluate_without_bound_sender_stays_bare_without_owner_shortcut() {
    let tmp = tempfile::TempDir::new().unwrap();
    let session_manager = session_manager_with_checkpoint_sender("s-plain", None).await;
    let adapter = adapter_with_ruleset(tmp.path(), all_deny_ruleset(), session_manager);
    let mut caller = PermCaller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
    };
    let verdict = adapter
        .evaluate(
            Some("s-plain"),
            &mut caller,
            &PermRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/data/file.txt".to_string(),
                op: "read".to_string(),
            },
        )
        .await;
    assert!(
        matches!(
            verdict,
            PermVerdict::Denied {
                ref reason,
                approval_request_id: None,
                ..
            } if reason == "no matching rule"
        ),
        "no bound sender → stays Bare, no Owner shortcut → defaults deny: {:?}",
        verdict
    );
    assert_eq!(
        caller.user_id, "",
        "caller must stay untouched when no sender is bound"
    );
}

/// session_id None：不走 session 分支，请求保持 Bare → 无 Owner
/// 短路，defaults（全 Deny）拒绝；caller 不被改写。
#[tokio::test]
async fn evaluate_without_session_stays_bare_without_owner_shortcut() {
    let tmp = tempfile::TempDir::new().unwrap();
    let adapter = adapter_with_ruleset(tmp.path(), all_deny_ruleset(), test_session_manager());
    let mut caller = PermCaller {
        user_id: "unbound".to_string(),
        agent: "test-agent".to_string(),
    };
    let verdict = adapter
        .evaluate(
            None,
            &mut caller,
            &PermRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/data/file.txt".to_string(),
                op: "read".to_string(),
            },
        )
        .await;
    assert!(
        matches!(
            verdict,
            PermVerdict::Denied {
                ref reason,
                approval_request_id: None,
                ..
            } if reason == "no matching rule"
        ),
        "no session → stays Bare, no Owner shortcut → defaults deny: {:?}",
        verdict
    );
    assert_eq!(
        caller.user_id, "unbound",
        "caller must stay untouched when no session is given"
    );
}

fn test_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &closeclaw_gateway::GatewayConfig::default(),
        None,
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ))
}
