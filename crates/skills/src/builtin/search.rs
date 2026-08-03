//! Search skill (web search)
use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;

#[derive(Default)]
pub struct SearchSkill;

impl SearchSkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// query is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "search",
            "description": "Web search capabilities",
            "supported_tools": ["web_search", "web_fetch"],
            "usage": {
                "query": "<search_query>",
                "url": "<url_to_fetch>"
            }
        })
        .to_string()
    }

    /// Build structured guidance for a search query.
    fn build_search_guidance(query: &str) -> String {
        json!({
            "skill": "search",
            "action": "search",
            "query": query,
            "guidance": "Use web_search tool with the provided query",
            "tools": ["web_search", "web_fetch"]
        })
        .to_string()
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

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        match args.get("query").and_then(|v| v.as_str()) {
            None => Ok(Self::capabilities_description()),
            Some(q) => Ok(Self::build_search_guidance(q)),
        }
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to search the \
                web for information or fetch content from URLs"
                .to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}
