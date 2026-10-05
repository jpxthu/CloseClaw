//! Behavior tests for agent lookup traits: `AgentConfigInfo` value semantics
//! and `AgentRegistryQuery` supertrait-combination contract (mock implements
//! all three supertraits, dispatched through `Arc<dyn AgentRegistryQuery>`).

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── AgentConfigInfo value semantics ───────────────────────────────────────

#[test]
fn test_agent_config_info_default_all_fields_none() {
    let info = AgentConfigInfo::default();
    assert_eq!(
        info.subagents_model, None,
        "default subagents_model must be None"
    );
    assert_eq!(
        info.timeout_warning, None,
        "default timeout_warning must be None"
    );
    assert_eq!(
        info.timeout_notify_interval_ratio, None,
        "default timeout_notify_interval_ratio must be None"
    );
}

#[test]
fn test_agent_config_info_clone_consistency() {
    let original = AgentConfigInfo {
        subagents_model: Some(ModelSpec::with_fallback("p/m", vec!["f/a".into()])),
        timeout_warning: Some(600),
        timeout_notify_interval_ratio: Some(0.5),
    };
    let cloned = original.clone();
    assert_eq!(cloned.subagents_model, original.subagents_model);
    assert_eq!(cloned.timeout_warning, original.timeout_warning);
    assert_eq!(
        cloned.timeout_notify_interval_ratio,
        original.timeout_notify_interval_ratio
    );

    // Clone 独立性: 修改 clone 不影响 original（值语义）
    let mut cloned = cloned;
    cloned.subagents_model = None;
    cloned.timeout_warning = None;
    assert!(
        original.subagents_model.is_some(),
        "clone independence: clearing cloned.subagents_model must not clear original.subagents_model"
    );
    assert_eq!(
        original.timeout_warning,
        Some(600),
        "clone independence: clearing cloned.timeout_warning must not clear original.timeout_warning"
    );
}

// ── AgentRegistryQuery 组合契约（mock 实现三 supertrait） ─────────────────
//
// 与 `crates/agent/src/registry/registry_query_tests.rs` 的分工：agent 侧
// 那组用例验证真实 `AgentRegistry` 实现三 supertrait 后经 `Arc<dyn
// AgentRegistryQuery>` 分发；common 不能依赖 agent crate，故此处仅在 trait
// 定义所在 crate 用 mock 复核「三 supertrait 组合后可作为组合 trait 对象使用」
// 的接口契约。本文件是该契约的定义侧验证（mock 即契约的标准样例），agent 侧
// 用例是实现侧验证，两组用例互不替代，均保留。

#[derive(Clone, Debug)]
struct MockAgentRecord {
    model: Option<ModelSpec>,
    workspace: Option<PathBuf>,
    bootstrap_mode: BootstrapMode,
    skills: Option<Vec<String>>,
    tools: Option<AgentToolsConfig>,
}

#[derive(Default)]
struct MockRegistryQuery {
    agents: Mutex<HashMap<String, MockAgentRecord>>,
}

impl MockRegistryQuery {
    fn record(&self, agent_id: &str) -> Option<MockAgentRecord> {
        self.agents.lock().unwrap().get(agent_id).cloned()
    }

    fn insert(&self, agent_id: &str, record: MockAgentRecord) {
        self.agents
            .lock()
            .unwrap()
            .insert(agent_id.to_string(), record);
    }
}

#[async_trait::async_trait]
impl AgentLookup for MockRegistryQuery {
    async fn get_agent_model(&self, agent_id: &str) -> Option<ModelSpec> {
        self.record(agent_id)?.model
    }

    async fn agent_exists(&self, agent_id: &str) -> bool {
        self.agents.lock().unwrap().contains_key(agent_id)
    }

    async fn query_bootstrap_mode(&self, agent_id: &str) -> Option<BootstrapMode> {
        self.record(agent_id).map(|r| r.bootstrap_mode)
    }

    async fn get_agent_workspace(&self, agent_id: &str) -> Option<PathBuf> {
        self.record(agent_id)?.workspace
    }
}

impl AgentSkillsQuery for MockRegistryQuery {
    fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>> {
        self.record(agent_id)?.skills
    }
}

#[async_trait::async_trait]
impl AgentToolsConfigQuery for MockRegistryQuery {
    async fn get_agent_tools_config(&self, agent_id: &str) -> Option<AgentToolsConfig> {
        self.record(agent_id)?.tools
    }
}

impl AgentRegistryQuery for MockRegistryQuery {}

fn sample_agent_record() -> MockAgentRecord {
    MockAgentRecord {
        model: Some(ModelSpec::single("gpt-4o")),
        workspace: Some(PathBuf::from("/workspace")),
        bootstrap_mode: BootstrapMode::Minimal,
        skills: Some(vec!["coding".to_string()]),
        tools: Some(AgentToolsConfig {
            tools: Some(vec!["read".to_string()]),
            disallowed_tools: Some(vec!["exec".to_string()]),
        }),
    }
}

// 组合可达: 已注册 agent 经 Arc<dyn AgentRegistryQuery> 三组方法集全部可达
#[tokio::test]
async fn test_agent_registry_query_dispatches_all_supertrait_methods() {
    let mock = Arc::new(MockRegistryQuery::default());
    mock.insert("a1", sample_agent_record());
    let q: Arc<dyn AgentRegistryQuery> = mock;

    assert_eq!(
        q.get_agent_model("a1").await,
        Some(ModelSpec::single("gpt-4o"))
    );
    assert!(
        q.agent_exists("a1").await,
        "registered agent 'a1' must be reachable via Arc<dyn AgentRegistryQuery>"
    );
    assert_eq!(
        q.query_bootstrap_mode("a1").await,
        Some(BootstrapMode::Minimal)
    );
    assert_eq!(
        q.get_agent_workspace("a1").await,
        Some(PathBuf::from("/workspace"))
    );
    assert_eq!(q.get_agent_skills("a1"), Some(vec!["coding".to_string()]));
    let tools = q.get_agent_tools_config("a1").await.unwrap();
    assert_eq!(tools.tools, Some(vec!["read".to_string()]));
    assert_eq!(tools.disallowed_tools, Some(vec!["exec".to_string()]));
}

// 组合边界: 不存在的 agent → 三组方法集全部回落默认（None / false）
#[tokio::test]
async fn test_agent_registry_query_missing_agent_defaults() {
    let mock = Arc::new(MockRegistryQuery::default());
    let q: Arc<dyn AgentRegistryQuery> = mock;

    assert!(
        !q.agent_exists("ghost").await,
        "unregistered agent 'ghost' must not be reported as existing"
    );
    assert_eq!(q.get_agent_model("ghost").await, None);
    assert_eq!(q.query_bootstrap_mode("ghost").await, None);
    assert_eq!(q.get_agent_workspace("ghost").await, None);
    assert_eq!(q.get_agent_skills("ghost"), None);
    assert_eq!(q.get_agent_tools_config("ghost").await, None);
}

// 状态转换: 注册前不可见、注册后经组合 trait 对象立即可见
#[tokio::test]
async fn test_agent_registry_query_reflects_state_transitions() {
    let mock = Arc::new(MockRegistryQuery::default());
    let q: Arc<dyn AgentRegistryQuery> = mock.clone();

    assert!(
        !q.agent_exists("a2").await,
        "agent 'a2' must be invisible before registration"
    );
    assert_eq!(q.get_agent_skills("a2"), None);
    assert_eq!(q.get_agent_tools_config("a2").await, None);

    mock.insert("a2", sample_agent_record());

    assert!(
        q.agent_exists("a2").await,
        "agent 'a2' must become visible immediately after registration"
    );
    assert_eq!(
        q.get_agent_model("a2").await,
        Some(ModelSpec::single("gpt-4o"))
    );
    assert_eq!(q.get_agent_skills("a2"), Some(vec!["coding".to_string()]));
    assert!(
        q.get_agent_tools_config("a2").await.is_some(),
        "tools config registered for 'a2' must be reachable through the combined trait object"
    );
}
