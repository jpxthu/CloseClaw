//! Search skill (web search)
use crate::skills::{Skill, SkillError, SkillManifest};
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

    fn methods(&self) -> Vec<&str> {
        vec!["search"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("query required".to_string()))?;
                Ok(serde_json::json!({
                    "query": query,
                    "results": [],
                    "is_stub": true,
                    "message": "Search skill stub - integrate with search API"
                }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "search".to_string(),
                method: method.to_string(),
            }),
        }
    }
}
