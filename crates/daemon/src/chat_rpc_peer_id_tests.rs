//! UT for the chat RPC peer_id contract (issue #3943, plan Step 1.3).
//!
//! Split from `chat_rpc_tests.rs` (file line-count hard limit 1000).
//! Behavior dimensions under test:
//! 1. Normal path: the peer_id produced by `build_inbound_input` /
//!    `build_stop_input` is the request's target agent identifier
//!    (non-constant value guards against constant-value regression,
//!    approach mirrors #3946's `terminal_msg_with_peer`);
//! 2. Boundary/isolation: different agent_id → different session route
//!    keys (chat.md §Session and Agent designation), same agent_id is
//!    stable and consistent;
//! 3. dispatch integration: the message entering the inbound chain has
//!    peer_id == agent_id (observed via RawLog raw-log files), and the
//!    `metadata["agent_id"]` attach still exists (explicit agent_id
//!    overrides bot→Agent bindings, the two mechanisms coexist);
//! 4. StopSession path: the peer_id of the `/stop` message is likewise
//!    the target agent and lands in the same session route key.

use super::*;
use crate::test_helpers::apply_test_sentinels;
use closeclaw_gateway::types::GatewayConfig;
use closeclaw_gateway::SessionManager;

/// Non-constant target agent values (unrelated to the historical
/// hardcode "cli"): a constant-value regression (any hardcode) fails on
/// the `assert_ne` / dual-value assertions.
const TEST_AGENT_X: &str = "agent-x";
const TEST_AGENT_Y: &str = "agent-y";

/// Dimension 1 — the peer_id of `build_inbound_input` is the request's
/// target agent identifier (chat.md:36: peer_id = target Agent identifier,
/// the session peer anchor).
#[test]
fn test_build_inbound_input_peer_id_is_target_agent() {
    let x = build_inbound_input(TEST_AGENT_X, "hello".to_string());
    let y = build_inbound_input(TEST_AGENT_Y, "hello".to_string());
    assert_eq!(x.peer_id, TEST_AGENT_X, "peer_id must be the target agent");
    assert_eq!(
        y.peer_id, TEST_AGENT_Y,
        "peer_id must track the request's agent_id"
    );
    assert_ne!(
        x.peer_id, y.peer_id,
        "different agents must not collide (guards a constant-value regression)"
    );
}

/// Dimension 1/4 — the peer_id of `build_stop_input` is likewise the
/// request's target agent; sender_id and /stop content pass through unchanged.
#[test]
fn test_build_stop_input_peer_id_is_target_agent() {
    let x = build_stop_input(TEST_AGENT_X, "sender-1".to_string());
    let y = build_stop_input(TEST_AGENT_Y, "sender-1".to_string());
    assert_eq!(
        x.peer_id, TEST_AGENT_X,
        "stop peer_id must be the target agent"
    );
    assert_eq!(
        y.peer_id, TEST_AGENT_Y,
        "stop peer_id must track the request's agent_id"
    );
    assert_ne!(x.peer_id, y.peer_id);
    assert_eq!(
        x.sender_id, "sender-1",
        "stop sender_id must pass through unchanged"
    );
    assert_eq!(x.content, "/stop");
}

/// Dimension 2 — the inbound messages constructed by `build_inbound_input` are substituted
/// field-by-field into the `compute_session_key` signature (platform + sender_id +
/// peer_id + account_id): different agent_id → different session route keys (isolation),
/// same agent_id → stable and consistent. The timestamp is fixed to eliminate its effect
/// on equality.
#[test]
fn test_inbound_session_key_isolates_per_agent() {
    let key_for = |agent: &str| {
        let msg = build_inbound_input(agent, "hello".to_string());
        closeclaw_common::session_key::compute_session_key(
            &msg.platform,
            &msg.sender_id,
            &msg.peer_id,
            Some(&msg.account_id),
            1_729_670_400_000,
        )
    };
    assert_ne!(
        key_for(TEST_AGENT_X),
        key_for(TEST_AGENT_Y),
        "different Agent conversations must land in different session route keys \
         (chat.md:36 isolation semantics)"
    );
    assert_eq!(
        key_for(TEST_AGENT_X),
        key_for(TEST_AGENT_X),
        "same agent_id route key must be stable and consistent"
    );
}

/// Dimension 2/4 — /stop must land in the same agent session:
/// in the production call (`dispatch_stop_session` passes current_uid),
/// `build_stop_input` and `build_inbound_input` are isomorphic across the four
/// route-key field dimensions.
#[test]
fn test_stop_input_matches_inbound_session_route_fields() {
    let uid = closeclaw_platform::current_uid();
    let inbound = build_inbound_input(TEST_AGENT_X, "hello".to_string());
    let stop = build_stop_input(TEST_AGENT_X, uid.clone());
    let route = |m: &NormalizedMessage| {
        (
            m.platform.clone(),
            m.sender_id.clone(),
            m.peer_id.clone(),
            m.account_id.clone(),
        )
    };
    assert_eq!(
        route(&inbound),
        route(&stop),
        "/stop and chat must resolve the same per-agent session route key"
    );
}

/// dispatch integration harness: assemble a [`ChatContext`] with the production processor
/// chain (RawLog → SessionRouter → ContentNormalizer, same assembly as
/// `crate::processor_registry`), and point raw_log_dir at a temporary directory —
/// the raw-log files are the observation point for "the message entering the inbound chain".
fn make_chain_dispatch_context(
    raw_log_dir: &Path,
    bindings: HashMap<String, String>,
) -> ChatContext {
    let config = apply_test_sentinels(GatewayConfig {
        raw_log_dir: Some(raw_log_dir.to_path_buf()),
        bot_agent_bindings: bindings,
        ..Default::default()
    });
    let sessions = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let chain = crate::processor_registry::build_processor_chain(&config);
    let gateway = Arc::new(Gateway::with_processor_registry(config, sessions, chain));
    ChatContext {
        gateway,
        rpc_plugin: Arc::new(RpcTerminalPlugin::new()),
    }
}

/// Read the inbound raw-log files produced by RawLogProcessor
/// (`{platform}_{timestamp}.json`; distinguish them from outbound logs by
/// the `_outbound_` marker) and restore them into [`NormalizedMessage`].
fn read_inbound_raw_logs(dir: &Path) -> Vec<NormalizedMessage> {
    let mut msgs = Vec::new();
    for entry in std::fs::read_dir(dir).expect("raw log dir must exist") {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || name.contains("_outbound_")
        {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("read raw log");
        msgs.push(
            serde_json::from_str(&raw).expect("raw log must deserialize to NormalizedMessage"),
        );
    }
    assert!(
        !msgs.is_empty(),
        "inbound chain raw log must have recorded the message"
    );
    msgs
}

/// Dimension 3 — dispatch handles `ChatMessage { agent_id }`:
/// ① the message entering the inbound chain (recorded by RawLog) has
///    peer_id == requested agent_id (a hardcode regression back to "cli" fails
///    on this assertion);
/// ② `metadata["agent_id"]` attach still exists — configure a bot→Agent binding
///    ("agent-x" → "bound-agent"); the session's agent_id must be the requested
///    value rather than the binding target (explicit agent_id takes priority),
///    proving the peer_id and metadata attach mechanisms coexist.
#[tokio::test]
async fn test_dispatch_chat_message_reaches_inbound_chain_with_target_agent_peer() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let bindings = HashMap::from([(TEST_AGENT_X.to_string(), "bound-agent".to_string())]);
    let context = make_chain_dispatch_context(tmp.path(), bindings);

    tokio::time::timeout(
        Duration::from_secs(1),
        dispatch(
            ChatRequest::ChatMessage {
                agent_id: TEST_AGENT_X.to_string(),
                content: "hello".to_string(),
            },
            &context,
        ),
    )
    .await
    .expect("dispatch must terminate (no session handler, returns immediately)");

    let logs = read_inbound_raw_logs(tmp.path());
    assert_eq!(logs.len(), 1, "exactly one inbound raw log");
    assert_eq!(logs[0].platform, "terminal");
    assert_eq!(
        logs[0].peer_id, TEST_AGENT_X,
        "the message entering the inbound chain must carry peer_id == requested agent_id"
    );
    assert_eq!(logs[0].content, "hello");

    let sessions = context.gateway.session_manager().get_all_sessions().await;
    assert!(
        sessions
            .iter()
            .any(|s| s.agent_id == TEST_AGENT_X && s.channel == "terminal"),
        "session must resolve under the requested agent_id \
         (metadata attach takes priority over binding), got {sessions:?}"
    );
}

/// Dimension 4 — the StopSession path: the message entering the inbound chain
/// after dispatch handles `StopSession { agent_id }` is the /stop message,
/// with peer_id == requested agent_id.
#[tokio::test]
async fn test_dispatch_stop_session_reaches_inbound_chain_with_target_agent_peer() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let context = make_chain_dispatch_context(tmp.path(), HashMap::new());

    tokio::time::timeout(
        Duration::from_secs(1),
        dispatch(
            ChatRequest::StopSession {
                agent_id: TEST_AGENT_X.to_string(),
            },
            &context,
        ),
    )
    .await
    .expect("dispatch must terminate (no slash dispatcher, returns immediately)");

    let logs = read_inbound_raw_logs(tmp.path());
    assert_eq!(logs.len(), 1, "exactly one inbound raw log");
    assert_eq!(
        logs[0].peer_id, TEST_AGENT_X,
        "stop message peer_id must be the requested agent_id"
    );
    assert_eq!(logs[0].content, "/stop");
}
