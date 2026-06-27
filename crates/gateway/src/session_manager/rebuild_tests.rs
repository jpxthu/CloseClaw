use super::flush_tests::{test_config, test_message};
use super::*;
use closeclaw_common::system_prompt::invalidate_all_sections;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use std::io::Write;
use tempfile::TempDir;

fn clear_global_prompt_state() {
    invalidate_all_sections();
}

fn make_temp_workspace(files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    for (name, content) in files {
        let path = tmp.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }
    tmp
}

fn make_test_mgr(workspace: Option<&std::path::Path>) -> SessionManager {
    SessionManager::new(
        &test_config(),
        None,
        workspace.map(std::path::PathBuf::from),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.4 — rebuild_system_prompt unit tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rebuild_system_prompt_normal() {
    clear_global_prompt_state();
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "original agents"),
        ("SOUL.md", "original soul"),
    ]);
    let mgr = make_test_mgr(Some(tmp.path()));
    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    {
        let conv = mgr.get_conversation_session(&session_id).await.unwrap();
        let conv = conv.read().await;
        assert!(conv.system_prompt().unwrap().contains("original agents"));
    }
    std::fs::write(tmp.path().join("AGENTS.md"), "updated agents").unwrap();
    mgr.rebuild_system_prompt(&session_id).await;
    {
        let conv = mgr.get_conversation_session(&session_id).await.unwrap();
        let conv = conv.read().await;
        assert!(conv.system_prompt().unwrap().contains("updated agents"));
    }
}

#[tokio::test]
async fn test_rebuild_system_prompt_edge_cases() {
    // nonexistent session — should not panic
    let mgr = make_test_mgr(None);
    mgr.rebuild_system_prompt("nonexistent-session-id").await;

    // no workspace — system prompt should only have tools + skills
    clear_global_prompt_state();
    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    mgr.rebuild_system_prompt(&session_id).await;
    {
        let conv = mgr.get_conversation_session(&session_id).await.unwrap();
        let conv = conv.read().await;
        let prompt = conv.system_prompt().unwrap();
        assert!(!prompt.contains("agents content"));
        assert!(!prompt.contains("soul content"));
        assert!(!prompt.contains("memory content"));
    }
}
