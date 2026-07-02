//! Provider for the Skills listing section of the system prompt.
//!
//! Delegates to [`DiskSkillRegistry::generate_listing`] and wraps the
//! result as a [`PromptFragment`].

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use closeclaw_skills::DiskSkillRegistry;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes the skill listing to the system prompt.
///
/// Holds a reference to the [`DiskSkillRegistry`] and optional agent-level
/// skills whitelist. When the registry is not initialised or produces an
/// empty listing, [`generate`](Self::generate) returns `None`.
pub struct SkillsFragmentProvider {
    registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Agent-level skill whitelist from config (`skills` field).
    ///
    /// When set, only skills whose names appear in the list are included.
    /// A value of `["*"]` means no filtering (all skills shown).
    agent_skills: Option<Vec<String>>,
}

impl SkillsFragmentProvider {
    pub fn new(
        registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
        agent_skills: Option<Vec<String>>,
    ) -> Self {
        Self {
            registry,
            agent_skills,
        }
    }
}

#[async_trait]
impl PromptFragmentProvider for SkillsFragmentProvider {
    fn name(&self) -> &str {
        "skills"
    }

    fn priority(&self) -> u32 {
        3
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let lock = self.registry.read().ok()?;
        let reg = lock.as_ref()?;

        let listing = reg.generate_listing(ctx.agent_id.as_deref(), self.agent_skills.as_deref());

        if listing.is_empty() {
            return None;
        }

        Some(PromptFragment {
            title: "## Available Skills".to_string(),
            section_type: SectionType::Skills,
            content: listing,
        })
    }

    /// Registry-backed — no file mtime to key on.
    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name_and_priority() {
        let reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let provider = SkillsFragmentProvider::new(reg, None);
        assert_eq!(provider.name(), "skills");
        assert_eq!(provider.priority(), 3);
    }

    #[test]
    fn test_cache_key_always_none() {
        let reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let provider = SkillsFragmentProvider::new(reg, None);
        let ctx = FragmentContext::default();
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[tokio::test]
    async fn test_generate_empty_registry_returns_none() {
        let reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let provider = SkillsFragmentProvider::new(reg, None);
        let ctx = FragmentContext::default();
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_none_inner_returns_none() {
        let reg = Arc::new(RwLock::<Option<DiskSkillRegistry>>::new(None));
        let provider = SkillsFragmentProvider::new(reg, None);
        let ctx = FragmentContext::default();
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_with_skills() {
        use closeclaw_skills::disk::{DiskSkill, SkillManifest, SkillSource};
        let skills = vec![DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: "test-skill".into(),
                description: "A test skill".into(),
                allowed_tools: vec![],
                when_to_use: "when testing".into(),
                context: Default::default(),
                agent: String::new(),
                agent_id: String::new(),
                effort: Default::default(),
                paths: vec![],
                user_invocable: true,
            },
            readme_path: std::path::PathBuf::from("/skills/test-skill/SKILL.md"),
            skill_dir: std::path::PathBuf::from("/skills/test-skill"),
            body: String::new(),
        }];
        let reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(skills))));
        let provider = SkillsFragmentProvider::new(reg, None);
        let ctx = FragmentContext::default();
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.section_type, SectionType::Skills);
        assert_eq!(frag.title, "## Available Skills");
        assert!(frag.content.contains("test-skill"));
    }

    #[tokio::test]
    async fn test_generate_with_agent_filter() {
        use closeclaw_skills::disk::{DiskSkill, SkillManifest, SkillSource};
        let skills = vec![
            DiskSkill {
                source: SkillSource::Bundled,
                manifest: SkillManifest {
                    name: "skill-a".into(),
                    description: "Skill A".into(),
                    allowed_tools: vec![],
                    when_to_use: String::new(),
                    context: Default::default(),
                    agent: String::new(),
                    agent_id: "agent1".into(),
                    effort: Default::default(),
                    paths: vec![],
                    user_invocable: true,
                },
                readme_path: std::path::PathBuf::from("/skills/skill-a/SKILL.md"),
                skill_dir: std::path::PathBuf::from("/skills/skill-a"),
                body: String::new(),
            },
            DiskSkill {
                source: SkillSource::Bundled,
                manifest: SkillManifest {
                    name: "skill-b".into(),
                    description: "Skill B".into(),
                    allowed_tools: vec![],
                    when_to_use: String::new(),
                    context: Default::default(),
                    agent: String::new(),
                    agent_id: "agent2".into(),
                    effort: Default::default(),
                    paths: vec![],
                    user_invocable: true,
                },
                readme_path: std::path::PathBuf::from("/skills/skill-b/SKILL.md"),
                skill_dir: std::path::PathBuf::from("/skills/skill-b"),
                body: String::new(),
            },
        ];
        let reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(skills))));
        let provider = SkillsFragmentProvider::new(reg, None);
        let ctx = FragmentContext {
            agent_id: Some("agent1".to_string()),
            ..Default::default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert!(frag.content.contains("skill-a"));
        assert!(!frag.content.contains("skill-b"));
    }
}
