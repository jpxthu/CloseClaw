use super::*;
use crate::GatewayConfig;
use closeclaw_common::{
    PendingMessage, PlanState, SessionLookup, SessionMode, ToolDescriptor, ToolFlags,
};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::collections::HashMap;

// ── Mock persistence (checkpoint / sender_id lookups) ─────────────────

struct MockPersist(tokio::sync::Mutex<HashMap<String, SessionCheckpoint>>);

impl MockPersist {
    fn new() -> Self {
        Self(tokio::sync::Mutex::new(HashMap::new()))
    }

    async fn put(&self, cp: &SessionCheckpoint) {
        self.0
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.put(cp).await;
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.0.lock().await.get(sid).cloned())
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

fn make_session_manager(persist: &Arc<MockPersist>) -> SessionManager {
    SessionManager::new(
        &GatewayConfig::default(),
        Some(Arc::clone(persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    )
}

/// SessionManager whose checkpoint for `sid` records `sender` as sender_id.
async fn sm_with_sender(sid: &str, sender: Option<&str>) -> SessionManager {
    let persist = Arc::new(MockPersist::new());
    let mut cp = SessionCheckpoint::new(sid.to_string());
    cp.sender_id = sender.map(str::to_string);
    persist.put(&cp).await;
    make_session_manager(&persist)
}

fn tool_ctx(session_id: Option<&str>) -> ToolContext {
    ToolContext {
        agent_id: "master".to_string(),
        workdir: None,
        session_id: session_id.map(str::to_string),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

// ── Dimension ③: user_id source = checkpoint sender, never session id ─

/// The resolved caller user_id is the checkpoint's recorded sender;
/// the session id is only the lookup key, never the identity.
#[tokio::test]
async fn test_resolve_caller_user_id_is_checkpoint_sender_not_session_id() {
    let sid = "sess-wiring-source";
    let sm = sm_with_sender(sid, Some("u_1001")).await;
    let uid = resolve_caller_user_id(&sm, &tool_ctx(Some(sid))).await;
    assert_eq!(uid, "u_1001");
    assert_ne!(uid, sid, "session id must never be used as user_id");
}

/// A recorded "owner" sender passes through, keeping the design
/// Owner shortcut ("CLI 调用默认为 Owner") reachable from dispatch.
#[tokio::test]
async fn test_resolve_caller_user_id_passes_owner_through() {
    let sid = "sess-owner-sender";
    let sm = sm_with_sender(sid, Some("owner")).await;
    let uid = resolve_caller_user_id(&sm, &tool_ctx(Some(sid))).await;
    assert_eq!(uid, "owner");
}

// ── Dimension ④: fallback → empty user_id ─────────────────────────────

/// No checkpoint / checkpoint without sender_id / missing or empty
/// session id all fall back to an empty user_id (engine empty-uid
/// branch: User phase skipped, Agent dimension decides alone).
#[tokio::test]
async fn test_resolve_caller_user_id_falls_back_to_empty() {
    let sm = sm_with_sender("sess-no-sender", None).await;
    let no_sender = resolve_caller_user_id(&sm, &tool_ctx(Some("sess-no-sender"))).await;
    assert_eq!(no_sender, "", "checkpoint without sender_id");
    let no_ckpt = resolve_caller_user_id(&sm, &tool_ctx(Some("sess-unknown"))).await;
    assert_eq!(no_ckpt, "", "session without checkpoint");
    let no_sid = resolve_caller_user_id(&sm, &tool_ctx(None)).await;
    assert_eq!(no_sid, "", "missing session_id");
    let empty_sid = resolve_caller_user_id(&sm, &tool_ctx(Some(""))).await;
    assert_eq!(empty_sid, "", "empty session_id");
}

// ── Executor-level integration: PermissionRequest built from the wiring ─

struct ProbeRegistry;

#[async_trait::async_trait]
impl ToolRegistryQuery for ProbeRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        vec!["probe_tool".to_string()]
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor> {
        vec![]
    }
    async fn has_tool(&self, name: &str) -> bool {
        name == "probe_tool"
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
    async fn get_tool_detail(&self, name: &str) -> Option<ToolDescriptor> {
        (name == "probe_tool").then(|| ToolDescriptor {
            name: "probe_tool".to_string(),
            group: "test_tools".to_string(),
            summary: String::new(),
            detail: String::new(),
            input_schema: serde_json::json!({}),
            flags: ToolFlags {
                is_concurrency_safe: true,
                is_read_only: true,
                is_destructive: false,
                is_expensive: false,
                is_deferred_by_default: false,
            },
        })
    }
    async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
        vec![]
    }
    async fn get_tool_concurrency_safe(&self, _name: &str) -> Option<bool> {
        Some(true)
    }
    async fn call_tool(
        &self,
        _name: &str,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        Ok(ToolResult {
            data: serde_json::json!({"content": "PROBE_OK"}),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

struct ProbeLookup;

#[async_trait::async_trait]
impl SessionLookup for ProbeLookup {
    async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
        None
    }
    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        None
    }
    async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
        Ok(())
    }
    async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
        None
    }
    async fn set_plan_state(&self, _: &str, _: PlanState) {}
    async fn set_session_mode(&self, _: &str, _: SessionMode) {}
}

/// Engine rules: Agent Allow + UserAndAgent("u_1001") Allow for
/// `test_tools::probe_tool`; everything else (Agent/User defaults)
/// Deny. So the tool only executes when the PermissionRequest's
/// caller user_id resolves to `u_1001` (or an empty uid — the
/// documented fallback); a leaked session id would miss the User
/// rule and hit user_defaults → Deny.
fn probe_ruleset() -> closeclaw_permission::RuleSet {
    serde_json::from_value::<closeclaw_permission::RuleSet>(serde_json::json!({
        "rules": [
            {
                "name": "agent-allow-probe",
                "subject": { "agent": "master" },
                "effect": "allow",
                "actions": [
                    { "type": "tool_call", "skill": "test_tools", "methods": ["probe_tool"] }
                ],
                "priority": 10,
            },
            {
                "name": "user-allow-probe",
                "subject": {
                    "match_mode": "user_and_agent",
                    "fields": {
                        "user_id": "u_1001",
                        "agent": "master",
                        "user_match": "exact",
                        "agent_match": "exact",
                    },
                },
                "effect": "allow",
                "actions": [
                    { "type": "tool_call", "skill": "test_tools", "methods": ["probe_tool"] }
                ],
                "priority": 10,
            },
        ],
    }))
    .expect("probe ruleset JSON is a valid RuleSet")
}

/// Run `probe_tool` through [`TraitObjectExecutor`] with the real
/// permission check wiring for a session whose checkpoint sender is
/// `sender` (`None` = checkpoint without sender_id).
async fn run_probe(sender: Option<&str>) -> ToolResult {
    use closeclaw_permission::engine::engine_types as et;

    let sid = "sess-probe";
    let sm = Arc::new(sm_with_sender(sid, sender).await);
    let engine =
        closeclaw_permission::PermissionEngine::new_with_default_data_root(probe_ruleset());
    let perm = Arc::new(tokio::sync::RwLock::new(engine));
    let config_dir = tempfile::tempdir().expect("config dir");
    let cm = Arc::new(
        closeclaw_config::manager::ConfigManager::new(config_dir.path().to_path_buf())
            .expect("ConfigManager::new"),
    );
    let af = Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::new(ProbeLookup) as Arc<dyn SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        config_dir.path().to_path_buf(),
        et::RuleSet::default(),
    )));
    let executor = TraitObjectExecutor::new(Arc::new(ProbeRegistry), tool_ctx(Some(sid)))
        .with_perm_deps((
            perm,
            sm,
            Arc::clone(&cm) as Arc<closeclaw_config::manager::ConfigManager>,
            af,
        ));
    let call = PendingToolCall {
        id: "call-1".to_string(),
        tool_name: "probe_tool".to_string(),
        args: serde_json::json!({}),
        file_path: None,
        is_concurrency_safe: true,
    };
    executor.execute(&call).await
}

/// ③ integration: the tool executes because the PermissionRequest
/// user_id resolved to the checkpoint sender `u_1001` — under the
/// old session-id wiring the User phase would miss and deny.
#[tokio::test]
async fn test_executor_tool_call_allowed_when_sender_matches_user_rule() {
    let result = run_probe(Some("u_1001")).await;
    let data = result.data.to_string();
    assert!(data.contains("PROBE_OK"), "expected tool run, got {data}");
}

/// ② integration: sender present but no User rule matches →
/// user_defaults Deny vetoes the Agent Allow (intersection model).
#[tokio::test]
async fn test_executor_tool_call_denied_when_sender_has_no_user_rule() {
    let result = run_probe(Some("u_9999")).await;
    let data = result.data.to_string();
    assert!(
        data.contains("not permitted"),
        "expected intersection denial, got {data}"
    );
}

/// ④ integration: sender missing → empty user_id → engine empty-uid
/// branch (User phase skipped, Agent Allow decides) → tool executes.
#[tokio::test]
async fn test_executor_tool_call_falls_back_to_agent_dimension_without_sender() {
    let result = run_probe(None).await;
    let data = result.data.to_string();
    assert!(
        data.contains("PROBE_OK"),
        "expected fallback allow, got {data}"
    );
}
