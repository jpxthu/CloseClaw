use super::session_handler::MessageMetadata;
use super::system_prompt_inject::{
    build_dynamic_sections, build_full_system_prompt, split_static_dynamic,
};
use super::*;
use crate::llm::client::UnifiedChatClient;
use crate::llm::fallback::FallbackClient;
use crate::llm::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use crate::llm::provider::{Provider, ProviderError};
use crate::llm::types::ProtocolId;
use crate::llm::types::{
    InternalRequest, InternalResponse, RawContentBlock, RawUsage, SseStateMachine,
};
use crate::llm::LLMRegistry;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::system_prompt::sections::Section;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;

#[derive(Debug, Clone)]
struct TestProvider {
    client: Client,
    headers: HeaderMap,
}
impl TestProvider {
    fn new() -> Self {
        Self {
            client: Client::new(),
            headers: HeaderMap::new(),
        }
    }
}
#[async_trait]
impl Provider for TestProvider {
    fn id(&self) -> &str {
        "test"
    }
    fn base_url(&self) -> &str {
        "http://localhost"
    }
    fn api_key(&self) -> &str {
        ""
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }
    fn http_client(&self) -> &Client {
        &self.client
    }
    fn default_headers(&self) -> &HeaderMap {
        &self.headers
    }
    async fn send(
        &self,
        _req: InternalRequest,
        _body: serde_json::Value,
    ) -> std::result::Result<InternalResponse, ProviderError> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("test".into())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
    async fn send_streaming(
        &self,
        _req: InternalRequest,
        _body: serde_json::Value,
    ) -> std::result::Result<crate::llm::provider::SseStream, ProviderError> {
        let (_, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
}

#[derive(Debug, Clone)]
struct TestProtocol {
    id: ProtocolId,
}
impl TestProtocol {
    fn new() -> Self {
        Self {
            id: ProtocolId::new("test"),
        }
    }
}
#[async_trait]
impl ChatProtocol for TestProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        "/chat"
    }
    fn build_request(
        &self,
        _req: &InternalRequest,
    ) -> crate::llm::protocol::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    fn parse_response(
        &self,
        _body: serde_json::Value,
    ) -> crate::llm::protocol::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("test".into())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
    fn decorate_headers(&self, _h: &mut HeaderMap) -> crate::llm::protocol::Result<()> {
        Ok(())
    }
    fn create_sse_machine(&self) -> SseStateMachine {
        SseStateMachine::new()
    }
    async fn parse_sse_stream(
        &self,
        _incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        Box::pin(futures::stream::empty())
    }
}

fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    let uc = Arc::new(UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(TestProvider::new()),
        Arc::new(TestProtocol::new()),
        Default::default(),
        Default::default(),
    ));
    SessionMessageHandler::new_no_output(sm, fallback, uc)
}

fn make_msg() -> crate::gateway::Message {
    use std::collections::HashMap;
    crate::gateway::Message {
        id: "msg_1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
    }
}

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
    }
}

/// build_dynamic_sections always includes ChannelContext with rendered fields.
#[test]
fn test_build_dynamic_sections_channel_context() {
    let meta = make_meta("user_42", "feishu", 1700000000);
    let sections = build_dynamic_sections(&meta, None, &[], None);

    // Find ChannelContext section
    let cc = sections
        .iter()
        .find(|s: &&Section| s.name() == "channel_context");
    assert!(cc.is_some(), "ChannelContext should always be present");
    let rendered = cc.unwrap().render();
    assert!(rendered.contains("sender_id: user_42"));
    assert!(rendered.contains("chat_name: feishu"));
    // Timestamp should be a valid RFC3339 string
    assert!(rendered.contains("timestamp:"));
}

/// session_timestamp parameter overrides meta.timestamp in ChannelContext.
#[test]
fn test_build_dynamic_sections_session_timestamp_override() {
    let meta = make_meta("user_42", "feishu", 1700000000);
    let session_ts: i64 = 1700000042;
    let sections = build_dynamic_sections(&meta, None, &[], Some(session_ts));
    let cc = sections
        .iter()
        .find(|s: &&Section| s.name() == "channel_context")
        .unwrap();
    let rendered = cc.render();
    // The timestamp should be derived from session_ts, not meta.timestamp
    let expected_dt = chrono::DateTime::from_timestamp(session_ts, 0).unwrap();
    assert!(
        rendered.contains(&expected_dt.to_rfc3339()),
        "ChannelContext timestamp should use session_timestamp when provided"
    );
}

/// When session_timestamp is None, ChannelContext falls back to meta.timestamp.
#[test]
fn test_build_dynamic_sections_session_timestamp_fallback() {
    let meta = make_meta("user_42", "feishu", 1700000000);
    let sections = build_dynamic_sections(&meta, None, &[], None);
    let cc = sections
        .iter()
        .find(|s: &&Section| s.name() == "channel_context")
        .unwrap();
    let rendered = cc.render();
    // The timestamp should be derived from meta.timestamp
    let expected_dt = chrono::DateTime::from_timestamp(meta.timestamp, 0).unwrap();
    assert!(
        rendered.contains(&expected_dt.to_rfc3339()),
        "ChannelContext timestamp should fallback to meta.timestamp when session_timestamp is None"
    );
}

/// build_dynamic_sections always includes SessionState with correct pending_tasks.
#[test]
fn test_build_dynamic_sections_session_state() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&meta, None, &[], None);

    let ss = sections
        .iter()
        .find(|s: &&Section| s.name() == "session_state");
    assert!(ss.is_some(), "SessionState should always be present");
    let rendered = ss.unwrap().render();
    assert!(rendered.contains("pending_tasks:"));
}

/// SessionState with empty pending_tasks.
#[test]
fn test_build_dynamic_sections_empty_pending_tasks() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&meta, None, &[], None);
    let ss = sections
        .iter()
        .find(|s: &&Section| s.name() == "session_state")
        .unwrap();
    assert!(ss.render().contains("pending_tasks:"));
}

/// AppendSection reflects the per-session system_appends slice:
/// absent when empty, formatted as a numbered `[N] 内容` list when
/// non-empty, and pushed as the last section in the list.
#[test]
fn test_build_dynamic_sections_append_section() {
    let meta = make_meta("u", "ch", 0);

    // Part 1: empty slice → no AppendSection
    let sections = build_dynamic_sections(&meta, None, &[], None);
    assert!(
        !sections.iter().any(|s| s.name() == "append"),
        "AppendSection absent when system_appends is empty"
    );

    // Part 2: non-empty slice → AppendSection with numbered list, last
    let items = vec![
        "first extra instruction".to_string(),
        "second extra instruction".to_string(),
    ];
    let sections2 = build_dynamic_sections(&meta, None, &items, None);
    let last = sections2.last().expect("sections should be non-empty");
    assert_eq!(
        last.name(),
        "append",
        "AppendSection must be the last section"
    );
    let rendered = last.render();
    assert!(
        rendered.contains("[0] first extra instruction"),
        "rendered should include [0] entry, got: {}",
        rendered
    );
    assert!(
        rendered.contains("[1] second extra instruction"),
        "rendered should include [1] entry, got: {}",
        rendered
    );
}

/// build_full_system_prompt composes static + boundary + dynamic sections.
#[test]
fn test_build_full_system_prompt_composition() {
    let meta = make_meta("alice", "telegram", 1700000000);
    let sections = build_dynamic_sections(&meta, None, &[], None);
    let full = build_full_system_prompt(Some("You are helpful."), &sections, None);

    // Contains static layer
    assert!(full.contains("You are helpful."));
    // Contains boundary marker
    assert!(full.contains("<!-- STATIC_LAYER_END -->"));
    // Contains dynamic ChannelContext
    assert!(full.contains("sender_id: alice"));
    // Contains dynamic SessionState
    assert!(full.contains("pending_tasks:"));
}

/// build_full_system_prompt with no static prompt uses only dynamic sections.
#[test]
fn test_build_full_system_prompt_no_static() {
    let meta = make_meta("bob", "ch", 0);
    let sections = build_dynamic_sections(&meta, None, &[], None);
    let full = build_full_system_prompt(None, &sections, None);

    // No boundary marker when no static prompt
    assert!(!full.contains("<!-- STATIC_LAYER_END -->"));
    // Still contains dynamic content
    assert!(full.contains("sender_id: bob"));
    assert!(full.contains("pending_tasks:"));
}

/// build_full_system_prompt with static but empty dynamic sections.
#[test]
fn test_build_full_system_prompt_empty_dynamic() {
    // Pass empty system_appends so dynamic sections are only ChannelContext + SessionState
    let meta = make_meta("", "", 0);
    // build_dynamic_sections always returns ChannelContext + SessionState (at minimum)
    let sections = build_dynamic_sections(&meta, None, &[], None);
    // These two sections always render to non-empty strings, so dynamic is never truly empty.
    // But we verify the composition still works.
    let full = build_full_system_prompt(Some("static"), &sections, None);
    assert!(full.contains("static"));
    assert!(full.contains("<!-- STATIC_LAYER_END -->"));
}

/// handle_message backward compat: returns LlmStarted for a normal idle session.
#[tokio::test]
async fn test_handle_message_backward_compat() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));

    // Original handle_message (no meta) should still return LlmStarted
    let result = handler.handle_message(&sid, "test input".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
    // Session should be busy immediately after
    assert!(sm.is_session_busy(&sid).await);

    // Wait for the async LLM task to finish (empty chain → failure → busy reset)
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    assert!(!sm.is_session_busy(&sid).await);
}

/// GitStatus section: build_dynamic_sections no longer reads global state.
/// Git status is tested via build_git_status_for directly.
#[test]
fn test_build_dynamic_sections_no_global_workdir() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&meta, None, &[], None);
    // Without a workdir_path parameter, no GitStatus section should appear
    let has_git = sections.iter().any(|s| s.name() == "git_status");
    assert!(!has_git, "GitStatus should not appear without workdir_path");
    assert!(!sections.is_empty());
}

/// When a workdir_path is provided, WorkingDirectory section is included.
#[test]
fn test_build_dynamic_sections_working_directory() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&meta, Some("/tmp/test"), &[], None);
    let wd = sections.iter().find(|s| s.name() == "working_directory");
    assert!(
        wd.is_some(),
        "WorkingDirectory should be present when workdir_path is Some"
    );
    let rendered = wd.unwrap().render();
    assert!(rendered.contains("## Working Directory"));
}

/// When a git repo path is provided, both WorkingDirectory and GitStatus appear.
#[test]
fn test_build_dynamic_sections_git_status_with_path() {
    let meta = make_meta("u", "ch", 0);
    // Use CARGO_MANIFEST_DIR which is a git repo
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let sections = build_dynamic_sections(&meta, Some(manifest_dir), &[], None);
    let has_wd = sections.iter().any(|s| s.name() == "working_directory");
    let has_git = sections.iter().any(|s| s.name() == "git_status");
    assert!(has_wd, "WorkingDirectory should be present");
    assert!(has_git, "GitStatus should be present for a git repo path");
}

/// When a non-git path is provided, WorkingDirectory appears but GitStatus does not.
#[test]
fn test_build_dynamic_sections_no_git_for_non_repo() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&meta, Some("/tmp"), &[], None);
    let has_wd = sections.iter().any(|s| s.name() == "working_directory");
    let has_git = sections.iter().any(|s| s.name() == "git_status");
    assert!(has_wd, "WorkingDirectory should be present");
    assert!(!has_git, "GitStatus should not appear for non-git path");
}

/// WorkingDirectory render sanitizes workspaces/ prefix to ~/.
#[test]
fn test_working_directory_render_sanitization() {
    let meta = make_meta("u", "ch", 0);
    // Use a path that contains workspaces/ to test sanitization
    let fake_workdir = "/home/user/.closeclaw/workspaces/agent1/user1/";
    let sections = build_dynamic_sections(&meta, Some(fake_workdir), &[], None);
    let wd = sections
        .iter()
        .find(|s| s.name() == "working_directory")
        .unwrap();
    let rendered = wd.render();
    assert!(
        rendered.contains("~/agent1/user1/"),
        "rendered should show sanitized path, got: {}",
        rendered
    );
    assert!(
        !rendered.contains(".closeclaw"),
        "rendered should not expose .closeclaw, got: {}",
        rendered
    );
}

/// workdir path with workspaces/ prefix does not expose config_dir root.
#[test]
fn test_workdir_path_no_config_dir_exposure() {
    use crate::system_prompt::sections::sanitize_workdir_path;
    let path = "/home/alice/.closeclaw/workspaces/my-agent/u123/";
    let sanitized = sanitize_workdir_path(path);
    assert_eq!(sanitized, "~/my-agent/u123/");
    assert!(!sanitized.contains(".closeclaw"));
    assert!(!sanitized.contains("/home/alice"));
}

// ═══════════════════════════════════════════════════════════════════════════
// split_static_dynamic Tests
// ═══════════════════════════════════════════════════════════════════════════

/// split_static_dynamic: content before marker → static, after → dynamic.
#[test]
fn test_split_static_dynamic_with_marker() {
    let input = "Be helpful.
<!-- STATIC_LAYER_END -->
You are a coding assistant.";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("Be helpful."));
    assert_eq!(d.as_deref(), Some("You are a coding assistant."));
}

/// split_static_dynamic: no marker → entire prompt goes to static.
#[test]
fn test_split_static_dynamic_no_marker() {
    let input = "You are a helpful assistant.";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("You are a helpful assistant."));
    assert_eq!(d, None);
}

/// split_static_dynamic: empty string → (None, None).
#[test]
fn test_split_static_dynamic_empty() {
    let (s, d) = split_static_dynamic("");
    assert_eq!(s, None);
    assert_eq!(d, None);
}

/// split_static_dynamic: marker at the very start → static is None, dynamic is the rest.
#[test]
fn test_split_static_dynamic_marker_at_start() {
    let input = "<!-- STATIC_LAYER_END -->\nSome dynamic content.";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s, None, "static should be None when marker is at the start");
    assert_eq!(d.as_deref(), Some("Some dynamic content."));
}

/// split_static_dynamic: marker at the very end → dynamic is None, static is the rest.
#[test]
fn test_split_static_dynamic_marker_at_end() {
    let input = "Static content.\n<!-- STATIC_LAYER_END -->";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("Static content."));
    assert_eq!(d, None, "dynamic should be None when marker is at the end");
}

/// split_static_dynamic: multiple markers → only the first one is used as the split point.
#[test]
fn test_split_static_dynamic_multiple_markers() {
    let input = "First part.\n<!-- STATIC_LAYER_END -->\nMiddle.\n<!-- STATIC_LAYER_END -->\nLast.";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("First part."));
    // Dynamic should contain everything after the first marker, including the second marker
    assert_eq!(
        d.as_deref(),
        Some("Middle.\n<!-- STATIC_LAYER_END -->\nLast.")
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Priority Prompt Override Tests (build_full_system_prompt)
// ═══════════════════════════════════════════════════════════════════════════
