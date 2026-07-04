//! Tests for `FragmentContext`, `PromptFragment`, and `SectionType`.

use super::*;
use crate::bootstrap::BootstrapMode;

// ---------------------------------------------------------------------------
// FragmentContext
// ---------------------------------------------------------------------------

#[test]
fn test_fragment_context_test_default() {
    let ctx = FragmentContext::test_default();
    assert_eq!(ctx.agent_id, "");
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
    assert!(ctx.workdir.is_dir());
}

#[test]
fn test_fragment_context_agent_id() {
    let ctx = FragmentContext {
        agent_id: "agent-42".to_string(),
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.agent_id, "agent-42");
}

#[test]
fn test_fragment_context_bootstrap_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
}

#[test]
fn test_fragment_context_workdir() {
    let ctx = FragmentContext {
        workdir: std::path::PathBuf::from("/tmp/workspace"),
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.workdir, std::path::PathBuf::from("/tmp/workspace"));
}

#[test]
fn test_fragment_context_all_fields() {
    let ctx = FragmentContext {
        agent_id: "my-agent".to_string(),
        bootstrap_mode: BootstrapMode::Minimal,
        workdir: std::path::PathBuf::from("/home/user/project"),
    };
    assert_eq!(ctx.agent_id, "my-agent");
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(ctx.workdir, std::path::PathBuf::from("/home/user/project"));
}

#[test]
fn test_fragment_context_clone() {
    let ctx = FragmentContext {
        agent_id: "clone-test".to_string(),
        bootstrap_mode: BootstrapMode::Minimal,
        workdir: std::path::PathBuf::from("/clone"),
    };
    let cloned = ctx.clone();
    assert_eq!(ctx.agent_id, cloned.agent_id);
    assert_eq!(ctx.bootstrap_mode, cloned.bootstrap_mode);
    assert_eq!(ctx.workdir, cloned.workdir);
}

#[test]
fn test_fragment_context_debug() {
    let ctx = FragmentContext {
        agent_id: "dbg".to_string(),
        ..FragmentContext::test_default()
    };
    let dbg = format!("{:?}", ctx);
    assert!(dbg.contains("dbg"));
}

#[test]
fn test_fragment_context_empty_agent_id_boundary() {
    let ctx = FragmentContext {
        agent_id: String::new(),
        ..FragmentContext::test_default()
    };
    assert!(ctx.agent_id.is_empty());
}

#[test]
fn test_fragment_context_minimal_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Minimal,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Minimal);
}

#[test]
fn test_fragment_context_full_mode() {
    let ctx = FragmentContext {
        bootstrap_mode: BootstrapMode::Full,
        ..FragmentContext::test_default()
    };
    assert_eq!(ctx.bootstrap_mode, BootstrapMode::Full);
}

// ---------------------------------------------------------------------------
// PromptFragmentProvider trait: mock implementation uses required fields
// ---------------------------------------------------------------------------

struct MockFragmentProvider;

#[async_trait]
impl PromptFragmentProvider for MockFragmentProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn priority(&self) -> u32 {
        100
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        // Uses all three required fields — must compile with non-Option types.
        if ctx.agent_id.is_empty() {
            return None;
        }
        Some(PromptFragment {
            section_title: format!("## Agent: {}", ctx.agent_id),
            section_type: SectionType::Bootstrap,
            content: format!(
                "mode={:?} dir={}",
                ctx.bootstrap_mode,
                ctx.workdir.display()
            ),
        })
    }

    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        Some(format!("mock:{}", ctx.agent_id))
    }
}

#[tokio::test]
async fn test_mock_provider_returns_none_for_empty_agent_id() {
    let provider = MockFragmentProvider;
    let ctx = FragmentContext::test_default(); // agent_id is empty
    assert!(provider.generate(&ctx).await.is_none());
}

#[tokio::test]
async fn test_mock_provider_generates_with_valid_fields() {
    let provider = MockFragmentProvider;
    let ctx = FragmentContext {
        agent_id: "test-agent".into(),
        bootstrap_mode: BootstrapMode::Minimal,
        workdir: std::path::PathBuf::from("/workspace"),
    };
    let frag = provider.generate(&ctx).await.unwrap();
    assert_eq!(frag.section_type, SectionType::Bootstrap);
    assert!(frag.section_title.contains("test-agent"));
    assert!(frag.content.contains("Minimal"));
    assert!(frag.content.contains("/workspace"));
}

#[test]
fn test_mock_provider_cache_key_includes_agent_id() {
    let provider = MockFragmentProvider;
    let ctx = FragmentContext {
        agent_id: "agent-99".into(),
        ..FragmentContext::test_default()
    };
    assert_eq!(provider.cache_key(&ctx).as_deref(), Some("mock:agent-99"));
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
