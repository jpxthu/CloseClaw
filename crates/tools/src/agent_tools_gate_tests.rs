//! Execution-time agent tools-config gate tests for `ToolRegistryImpl`.
//!
//! Compiled via `mod agent_tools_gate_tests;` inside `registry_tests.rs`
//! (itself attached to `registry.rs` through `#[path]`). Covers three layers:
//!
//! 1. `judge_agent_tool_allowed` — pure `AgentToolsConfigQuery` contract
//!    semantics applied at the tools (consumer) side.
//! 2. `check_agent_tool_allowed` — query consumption boundaries
//!    (not injected / agent not registered / canned configs).
//! 3. `call_tool` pre-call wiring — deny returns an observable deny
//!    `ToolResult` without ever touching `tool.call`; allowed calls
//!    pass through unchanged.
//!
//! Test red lines observed: no network, no env mutation, no sleeps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use closeclaw_common::tool_registry::ToolRegistryQuery;
use closeclaw_common::tool_trait::{ToolCallError, ToolContext, ToolFlags, ToolResult};
use closeclaw_common::{AgentToolsConfig, AgentToolsConfigQuery};

use crate::registry::{ToolRegistryImpl, AGENT_TOOLS_DENY_MARKER};
use crate::Tool;

/// Agent id carrying a deny-relevant config in wiring tests.
const AGENT_CFG: &str = "agent-cfg";

/// Tool name used by the probe tool and all configs.
const TOOL: &str = "Bash";

/// Canned-config `AgentToolsConfigQuery` mock keyed by agent id.
/// A missing id behaves like an unregistered agent (`None` → no filtering).
struct MapQuery {
    configs: HashMap<String, AgentToolsConfig>,
}

#[async_trait]
impl AgentToolsConfigQuery for MapQuery {
    async fn get_agent_tools_config(&self, agent_id: &str) -> Option<AgentToolsConfig> {
        self.configs.get(agent_id).cloned()
    }
}

/// Tool stub whose only behavior is counting `call` invocations.
struct CallProbeTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CallProbeTool {
    fn name(&self) -> &str {
        TOOL
    }
    fn group(&self) -> &str {
        "test_probe"
    }
    fn summary(&self) -> String {
        "call-counting probe".to_string()
    }
    fn detail(&self) -> String {
        "call-counting probe for gate wiring tests".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> ToolFlags {
        ToolFlags::default()
    }

    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            data: json!({ "ok": true }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

/// `Some(items)` shorthand so `None` vs `Some([])` stays explicit at call site.
fn some_list(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|s| (*s).to_string()).collect())
}

/// Run the pure judgment under test; `Ok(())` means the gate allows the tool.
fn judge_allows(
    tools: Option<Vec<String>>,
    denied: Option<Vec<String>>,
    tool: &str,
) -> Result<(), String> {
    ToolRegistryImpl::judge_agent_tool_allowed(&tools, &denied, tool)
}

/// Deny reason must name the tool and carry the observability marker.
fn assert_deny_shape(reason: &str) {
    assert!(reason.contains(TOOL), "reason must name tool: {reason}");
    assert!(
        reason.contains(AGENT_TOOLS_DENY_MARKER),
        "reason must carry marker: {reason}"
    );
}

fn make_ctx(agent_id: &str) -> ToolContext {
    ToolContext {
        agent_id: agent_id.to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

fn config_map(agent_id: &str, cfg: AgentToolsConfig) -> HashMap<String, AgentToolsConfig> {
    HashMap::from([(agent_id.to_string(), cfg)])
}

/// Bare registry with a `Bash` probe tool; returns the call counter.
async fn probe_registry() -> (ToolRegistryImpl, Arc<AtomicUsize>) {
    let reg = ToolRegistryImpl::new();
    let calls = Arc::new(AtomicUsize::new(0));
    reg.register(CallProbeTool {
        calls: Arc::clone(&calls),
    })
    .await
    .unwrap();
    (reg, calls)
}

/// Probe registry with injected canned agent configs.
async fn probe_registry_with(
    configs: HashMap<String, AgentToolsConfig>,
) -> (ToolRegistryImpl, Arc<AtomicUsize>) {
    let (reg, calls) = probe_registry().await;
    reg.set_agent_tools_query(Arc::new(MapQuery { configs }));
    (reg, calls)
}

/// Canned agent tools config (`tools` whitelist + `disallowedTools`
/// blacklist); named to avoid clashing with Rust attribute vocabulary.
fn make_agent_config(tools: Option<Vec<String>>, denied: Option<Vec<String>>) -> AgentToolsConfig {
    AgentToolsConfig {
        tools,
        disallowed_tools: denied,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. judge_agent_tool_allowed — pure contract semantics
// ═══════════════════════════════════════════════════════════════════════════

/// Normal: tool listed in a restricting whitelist is allowed.
#[tokio::test]
async fn test_judge_whitelist_membership_allows() {
    assert!(
        judge_allows(some_list(&[TOOL]), None, TOOL).is_ok(),
        "tool {TOOL} listed in the whitelist should pass the gate"
    );
}

/// Normal: tool outside a non-empty blacklist is allowed.
#[tokio::test]
async fn test_judge_tool_outside_blacklist_allows() {
    assert!(
        judge_allows(None, some_list(&["Write", "Edit"]), TOOL).is_ok(),
        "tool {TOOL} outside a non-empty blacklist should pass the gate"
    );
}

/// Normal: `"*"` wildcard whitelist leaves the tool unrestricted.
#[tokio::test]
async fn test_judge_wildcard_whitelist_allows() {
    assert!(
        judge_allows(some_list(&["*"]), None, TOOL).is_ok(),
        "wildcard whitelist should leave {TOOL} unrestricted"
    );
}

/// Normal: `(None, None)` — no config at all — is unrestricted.
#[tokio::test]
async fn test_judge_no_config_allows() {
    assert!(
        judge_allows(None, None, TOOL).is_ok(),
        "(None, None) config should leave {TOOL} unrestricted"
    );
}

/// Error: tool absent from a restricting whitelist is denied.
#[tokio::test]
async fn test_judge_tool_outside_whitelist_denied() {
    let reason = judge_allows(some_list(&["Read"]), None, TOOL).unwrap_err();
    assert_deny_shape(&reason);
}

/// Error: blacklist wins — tool listed in BOTH lists is still denied.
#[tokio::test]
async fn test_judge_blacklist_beats_whitelist() {
    let reason = judge_allows(some_list(&[TOOL, "Read"]), some_list(&[TOOL]), TOOL).unwrap_err();
    assert_deny_shape(&reason);
}

/// Boundary: `tools = Some([])` does not restrict.
#[tokio::test]
async fn test_judge_empty_whitelist_unrestricted() {
    assert!(
        judge_allows(some_list(&[]), None, TOOL).is_ok(),
        "empty whitelist should leave {TOOL} unrestricted"
    );
}

/// Boundary: `disallowed_tools = Some([])` does not restrict.
#[tokio::test]
async fn test_judge_empty_blacklist_unrestricted() {
    assert!(
        judge_allows(None, some_list(&[]), TOOL).is_ok(),
        "empty blacklist should leave {TOOL} unrestricted"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. check_agent_tool_allowed — query consumption boundaries
// ═══════════════════════════════════════════════════════════════════════════

/// Normal: whitelist listing the tool allows through the async check.
#[tokio::test]
async fn test_check_allows_whitelisted_tool() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&[TOOL]), None),
    ))
    .await;
    assert!(
        reg.check_agent_tool_allowed(AGENT_CFG, TOOL).await.is_ok(),
        "whitelisted tool {TOOL} should pass the gate"
    );
}

/// Normal: `"*"` wildcard config allows any tool.
#[tokio::test]
async fn test_check_allows_wildcard_config() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&["*"]), some_list(&[])),
    ))
    .await;
    assert!(
        reg.check_agent_tool_allowed(AGENT_CFG, TOOL).await.is_ok(),
        "wildcard config should let {TOOL} pass the gate"
    );
}

/// Error: blacklisted tool denied via query; reason carries both markers.
#[tokio::test]
async fn test_check_denies_blacklisted_tool() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&["*"]), some_list(&[TOOL])),
    ))
    .await;
    let reason = reg
        .check_agent_tool_allowed(AGENT_CFG, TOOL)
        .await
        .unwrap_err();
    assert_deny_shape(&reason);
}

/// Error: tool outside the whitelist denied via query.
#[tokio::test]
async fn test_check_denies_tool_outside_whitelist() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&["Read"]), None),
    ))
    .await;
    let reason = reg
        .check_agent_tool_allowed(AGENT_CFG, TOOL)
        .await
        .unwrap_err();
    assert_deny_shape(&reason);
}

/// Boundary: query never injected — unrestricted.
#[tokio::test]
async fn test_check_allows_when_query_not_injected() {
    let (reg, _) = probe_registry().await;
    assert!(
        reg.check_agent_tool_allowed(AGENT_CFG, TOOL).await.is_ok(),
        "no injected query should leave {TOOL} unrestricted"
    );
}

/// Boundary: injected query but agent id not registered — unrestricted.
#[tokio::test]
async fn test_check_allows_unregistered_agent() {
    let (reg, _) = probe_registry_with(config_map(
        "other-agent",
        make_agent_config(some_list(&["Read"]), some_list(&[TOOL])),
    ))
    .await;
    assert!(
        reg.check_agent_tool_allowed(AGENT_CFG, TOOL).await.is_ok(),
        "unregistered agent id should leave {TOOL} unrestricted"
    );
}

/// Boundary: `Some([])` on both lists via query — unrestricted.
#[tokio::test]
async fn test_check_allows_empty_vec_configs() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&[]), some_list(&[])),
    ))
    .await;
    assert!(
        reg.check_agent_tool_allowed(AGENT_CFG, TOOL).await.is_ok(),
        "Some([]) configs should leave {TOOL} unrestricted"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. call_tool pre-call wiring — deny shape, tool untouched, pass-through
// ═══════════════════════════════════════════════════════════════════════════

/// Wiring deny shape: `Ok(ToolResult)` whose `data.error` is a non-empty
/// string naming the tool and carrying the marker; no new messages; the
/// serialized `data` (downstream content payload) stays observable.
#[tokio::test]
async fn test_call_tool_deny_result_shape() {
    let (reg, _) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&["*"]), some_list(&[TOOL])),
    ))
    .await;
    let result = reg
        .call_tool(TOOL, json!({}), &make_ctx(AGENT_CFG))
        .await
        .unwrap();
    let error = result.data.get("error").and_then(Value::as_str);
    let error = error.expect("deny result must carry data.error string");
    assert!(!error.is_empty(), "data.error must be non-empty");
    assert_deny_shape(error);
    assert!(result.new_messages.is_empty(), "deny must add no messages");
    assert!(
        result.context_modifier.is_none(),
        "deny must not carry a context modifier"
    );
    let content = result.data.to_string();
    assert!(
        content.contains(AGENT_TOOLS_DENY_MARKER),
        "deny content must be observable: {content}"
    );
}

/// Wiring deny path never touches the tool: probe counter stays zero.
#[tokio::test]
async fn test_call_tool_deny_does_not_invoke_tool() {
    let (reg, calls) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&[TOOL]), some_list(&[TOOL])),
    ))
    .await;
    let result = reg
        .call_tool(TOOL, json!({}), &make_ctx(AGENT_CFG))
        .await
        .unwrap();
    assert!(
        result.data.get("error").is_some(),
        "deny result must carry data.error"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "tool.call must not run");
}

/// Wiring allow path: whitelisted tool executes and its own result passes
/// through unchanged (zero regression on the original call path).
#[tokio::test]
async fn test_call_tool_allows_whitelisted_tool() {
    let (reg, calls) = probe_registry_with(config_map(
        AGENT_CFG,
        make_agent_config(some_list(&[TOOL]), None),
    ))
    .await;
    let result = reg
        .call_tool(TOOL, json!({}), &make_ctx(AGENT_CFG))
        .await
        .unwrap();
    assert_eq!(result.data["ok"], json!(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// Wiring boundary: no query injected — call passes straight through.
#[tokio::test]
async fn test_call_tool_allows_when_query_not_injected() {
    let (reg, calls) = probe_registry().await;
    let result = reg
        .call_tool(TOOL, json!({}), &make_ctx(AGENT_CFG))
        .await
        .unwrap();
    assert_eq!(result.data["ok"], json!(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// Wiring boundary: unregistered agent — blacklist for another agent must
/// not leak onto this call.
#[tokio::test]
async fn test_call_tool_allows_unregistered_agent() {
    let (reg, calls) = probe_registry_with(config_map(
        "other-agent",
        make_agent_config(some_list(&[]), some_list(&[TOOL])),
    ))
    .await;
    let result = reg
        .call_tool(TOOL, json!({}), &make_ctx(AGENT_CFG))
        .await
        .unwrap();
    assert_eq!(result.data["ok"], json!(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
