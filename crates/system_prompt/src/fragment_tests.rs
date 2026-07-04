//! Tests for `FragmentContext`, `PromptFragment`, and `SectionType`.

use super::*;

// ---------------------------------------------------------------------------
// FragmentContext
// ---------------------------------------------------------------------------

#[test]
fn test_fragment_context_default() {
    let ctx = FragmentContext::default();
    assert!(ctx.agent_id.is_none());
    assert!(ctx.bootstrap_mode.is_none());
    assert!(ctx.workdir.is_none());
    assert!(ctx.agent_dir.is_none());
}

#[test]
fn test_fragment_context_agent_id() {
    let ctx = FragmentContext {
        agent_id: Some("agent-42".to_string()),
        ..Default::default()
    };
    assert_eq!(ctx.agent_id.as_deref(), Some("agent-42"));
}

#[test]
fn test_fragment_context_bootstrap_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: Some(BootstrapMode::Full),
        ..Default::default()
    };
    assert_eq!(ctx.bootstrap_mode, Some(BootstrapMode::Full));
}

#[test]
fn test_fragment_context_workdir() {
    let ctx = FragmentContext {
        workdir: Some(std::path::PathBuf::from("/tmp/workspace")),
        ..Default::default()
    };
    assert_eq!(
        ctx.workdir.as_ref().unwrap().to_str(),
        Some("/tmp/workspace")
    );
}

#[test]
fn test_fragment_context_all_fields() {
    let ctx = FragmentContext {
        agent_id: Some("my-agent".to_string()),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        workdir: Some(std::path::PathBuf::from("/home/user/project")),
        agent_dir: Some(std::path::PathBuf::from(
            "/home/user/.openclaw/agents/test-agent",
        )),
    };
    assert_eq!(ctx.agent_id.as_deref(), Some("my-agent"));
    assert_eq!(ctx.bootstrap_mode, Some(BootstrapMode::Minimal));
    assert!(ctx.workdir.is_some());
    assert_eq!(
        ctx.agent_dir.as_ref().unwrap().to_str(),
        Some("/home/user/.openclaw/agents/test-agent")
    );
}

#[test]
fn test_fragment_context_clone() {
    let ctx = FragmentContext {
        agent_id: Some("clone-test".to_string()),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        workdir: Some(std::path::PathBuf::from("/clone")),
        agent_dir: Some(std::path::PathBuf::from("/clone-agent")),
    };
    let cloned = ctx.clone();
    assert_eq!(ctx.agent_id, cloned.agent_id);
    assert_eq!(ctx.bootstrap_mode, cloned.bootstrap_mode);
    assert_eq!(ctx.workdir, cloned.workdir);
    assert_eq!(ctx.agent_dir, cloned.agent_dir);
}

#[test]
fn test_fragment_context_debug() {
    let ctx = FragmentContext {
        agent_id: Some("dbg".to_string()),
        ..Default::default()
    };
    let dbg = format!("{:?}", ctx);
    assert!(dbg.contains("dbg"));
}

// ---------------------------------------------------------------------------
// PromptFragment
// ---------------------------------------------------------------------------

#[test]
fn test_prompt_fragment_fields() {
    let frag = PromptFragment {
        section_title: "## Section".to_string(),
        section_type: SectionType::Tools,
        content: "content here".to_string(),
    };
    assert_eq!(frag.section_title, "## Section");
    assert_eq!(frag.section_type, SectionType::Tools);
    assert_eq!(frag.content, "content here");
}

#[test]
fn test_prompt_fragment_clone() {
    let frag = PromptFragment {
        section_title: "## Clone".to_string(),
        section_type: SectionType::Memory,
        content: "remember".to_string(),
    };
    let cloned = frag.clone();
    assert_eq!(frag.section_title, cloned.section_title);
    assert_eq!(frag.section_type, cloned.section_type);
    assert_eq!(frag.content, cloned.content);
}

#[test]
fn test_prompt_fragment_debug() {
    let frag = PromptFragment {
        section_title: "## Debug".to_string(),
        section_type: SectionType::Bootstrap,
        content: "debug content".to_string(),
    };
    let dbg = format!("{:?}", frag);
    assert!(dbg.contains("Bootstrap"));
    assert!(dbg.contains("debug content"));
}

// ---------------------------------------------------------------------------
// SectionType
// ---------------------------------------------------------------------------

#[test]
fn test_section_type_equality() {
    assert_eq!(SectionType::Bootstrap, SectionType::Bootstrap);
    assert_eq!(SectionType::Tools, SectionType::Tools);
    assert_eq!(SectionType::Skills, SectionType::Skills);
    assert_eq!(SectionType::Memory, SectionType::Memory);
    assert_ne!(SectionType::Bootstrap, SectionType::Tools);
    assert_ne!(SectionType::Skills, SectionType::Memory);
}

#[test]
fn test_section_type_clone() {
    let s = SectionType::Skills;
    let cloned = s;
    assert_eq!(s, cloned);
}

#[test]
fn test_section_type_debug() {
    assert_eq!(format!("{:?}", SectionType::Bootstrap), "Bootstrap");
    assert_eq!(format!("{:?}", SectionType::Tools), "Tools");
    assert_eq!(format!("{:?}", SectionType::Skills), "Skills");
    assert_eq!(format!("{:?}", SectionType::Memory), "Memory");
}
