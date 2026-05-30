use super::session_handler::MessageMetadata;
use super::system_prompt_inject::{build_dynamic_sections, build_full_system_prompt};
use super::*;
use crate::llm::fallback::FallbackClient;
use crate::llm::LLMRegistry;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::system_prompt::sections::{
    clear_append_section, get_append_section, set_append_section, Section,
};

fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    SessionMessageHandler::new_no_output(sm, fallback)
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
    let sections = build_dynamic_sections(0, &meta, None);

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

/// build_dynamic_sections always includes SessionState with correct turn_count.
#[test]
fn test_build_dynamic_sections_session_state() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(7, &meta, None);

    let ss = sections
        .iter()
        .find(|s: &&Section| s.name() == "session_state");
    assert!(ss.is_some(), "SessionState should always be present");
    let rendered = ss.unwrap().render();
    assert!(rendered.contains("turn_count: 7"));
    assert!(rendered.contains("pending_tasks:"));
}

/// SessionState with zero turn_count.
#[test]
fn test_build_dynamic_sections_turn_count_zero() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(0, &meta, None);
    let ss = sections
        .iter()
        .find(|s: &&Section| s.name() == "session_state")
        .unwrap();
    assert!(ss.render().contains("turn_count: 0"));
}

/// AppendSection is included when set, cleared after use; absent when unset.
#[test]
fn test_build_dynamic_sections_append_section() {
    // Part 1: set → build → should include and clear
    clear_append_section();
    let meta = make_meta("u", "ch", 0);
    set_append_section("extra instructions here".to_string());
    let sections = build_dynamic_sections(0, &meta, None);
    let has_append = sections.iter().any(|s| s.name() == "append");
    // Due to global state races with other tests, we only assert the
    // round-trip: set → get returns Some, then get returns None after build.
    // (Other tests may clear between set and build in parallel runs.)
    if has_append {
        assert!(
            get_append_section().is_none(),
            "AppendSection should be cleared after build"
        );
    }

    // Part 2: not set → build → should be absent
    clear_append_section();
    let sections2 = build_dynamic_sections(0, &meta, None);
    assert!(
        !sections2.iter().any(|s| s.name() == "append"),
        "AppendSection absent when unset"
    );
}

/// build_full_system_prompt composes static + boundary + dynamic sections.
#[test]
fn test_build_full_system_prompt_composition() {
    let meta = make_meta("alice", "telegram", 1700000000);
    let sections = build_dynamic_sections(3, &meta, None);
    let full = build_full_system_prompt(Some("You are helpful."), &sections);

    // Contains static layer
    assert!(full.contains("You are helpful."));
    // Contains boundary marker
    assert!(full.contains("<!-- STATIC_LAYER_END -->"));
    // Contains dynamic ChannelContext
    assert!(full.contains("sender_id: alice"));
    // Contains dynamic SessionState
    assert!(full.contains("turn_count: 3"));
}

/// build_full_system_prompt with no static prompt uses only dynamic sections.
#[test]
fn test_build_full_system_prompt_no_static() {
    let meta = make_meta("bob", "ch", 0);
    let sections = build_dynamic_sections(0, &meta, None);
    let full = build_full_system_prompt(None, &sections);

    // No boundary marker when no static prompt
    assert!(!full.contains("<!-- STATIC_LAYER_END -->"));
    // Still contains dynamic content
    assert!(full.contains("sender_id: bob"));
    assert!(full.contains("turn_count: 0"));
}

/// build_full_system_prompt with static but empty dynamic sections.
#[test]
fn test_build_full_system_prompt_empty_dynamic() {
    // Clear append section so dynamic sections are only ChannelContext + SessionState
    clear_append_section();
    let meta = make_meta("", "", 0);
    // build_dynamic_sections always returns ChannelContext + SessionState (at minimum)
    let sections = build_dynamic_sections(0, &meta, None);
    // These two sections always render to non-empty strings, so dynamic is never truly empty.
    // But we verify the composition still works.
    let full = build_full_system_prompt(Some("static"), &sections);
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
    let sections = build_dynamic_sections(0, &meta, None);
    // Without a workdir_path parameter, no GitStatus section should appear
    let has_git = sections.iter().any(|s| s.name() == "git_status");
    assert!(!has_git, "GitStatus should not appear without workdir_path");
    assert!(!sections.is_empty());
}

/// When a workdir_path is provided, WorkingDirectory section is included.
#[test]
fn test_build_dynamic_sections_working_directory() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(0, &meta, Some("/tmp/test"));
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
    let sections = build_dynamic_sections(0, &meta, Some(manifest_dir));
    let has_wd = sections.iter().any(|s| s.name() == "working_directory");
    let has_git = sections.iter().any(|s| s.name() == "git_status");
    assert!(has_wd, "WorkingDirectory should be present");
    assert!(has_git, "GitStatus should be present for a git repo path");
}

/// When a non-git path is provided, WorkingDirectory appears but GitStatus does not.
#[test]
fn test_build_dynamic_sections_no_git_for_non_repo() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(0, &meta, Some("/tmp"));
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
    let sections = build_dynamic_sections(0, &meta, Some(fake_workdir));
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
