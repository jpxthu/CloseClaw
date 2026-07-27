//! Search skill (web search)
use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillListingMeta, SkillManifest};
use async_trait::async_trait;

#[derive(Default)]
pub struct SearchSkill;

impl SearchSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for SearchSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "search".to_string(),
            version: "1.0.0".to_string(),
            description: "Web search capabilities".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Search Skill

Use the `web_search` tool to search the web for information. Provide a clear, concise query.

- For code-related searches, include the programming language and specific library/framework.
- For factual queries, include enough context to get precise results.
- Use `web_fetch` to retrieve full content from a specific URL when needed."#
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to search the web for information or fetch content from URLs".to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}
