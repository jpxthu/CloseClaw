//! Coding Agent Skill - Delegate coding tasks to AI coding agents
//!
//! This skill wraps OpenCode or Claude Code to handle complex coding tasks.

use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;

pub struct CodingAgentSkill {
    model: String,
}

impl CodingAgentSkill {
    pub fn new(model: Option<String>) -> Self {
        Self {
            model: model.unwrap_or_else(|| "minimax/MiniMax-M2.7".to_string()),
        }
    }
}

#[async_trait]
impl Skill for CodingAgentSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "coding_agent".to_string(),
            version: "1.0.0".to_string(),
            description:
                "Delegate complex coding tasks to AI coding agents (OpenCode, Claude Code)"
                    .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["delegate", "review", "refactor", "test"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "delegate" => Self::delegate(&args, &self.model),
            "review" => Self::review(&args),
            "refactor" => Self::refactor(&args),
            "test" => Self::test(&args),
            _ => Err(SkillError::MethodNotFound {
                skill: "coding_agent".to_string(),
                method: method.to_string(),
            }),
        }
    }
}

impl CodingAgentSkill {
    fn delegate(args: &serde_json::Value, model: &str) -> Result<serde_json::Value, SkillError> {
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("task required".to_string()))?;
        let language = args
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("rust");

        Ok(serde_json::json!({
            "status": "delegated",
            "task": task,
            "language": language,
            "model": model,
            "message": "Coding task delegated - implementation stub"
        }))
    }

    fn review(args: &serde_json::Value) -> Result<serde_json::Value, SkillError> {
        let _code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("code required".to_string()))?;

        Ok(serde_json::json!({
            "status": "review_complete",
            "issues": [],
            "message": "Code review - implementation stub"
        }))
    }

    fn refactor(args: &serde_json::Value) -> Result<serde_json::Value, SkillError> {
        let _code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("code required".to_string()))?;
        let goal = args
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("improve readability");

        Ok(serde_json::json!({
            "status": "refactored",
            "goal": goal,
            "message": "Refactoring - implementation stub"
        }))
    }

    fn test(args: &serde_json::Value) -> Result<serde_json::Value, SkillError> {
        let _code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("code required".to_string()))?;

        Ok(serde_json::json!({
            "status": "tests_generated",
            "test_count": 0,
            "message": "Test generation - implementation stub"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Constructor tests ===

    #[test]
    fn test_new_with_model() {
        let skill = CodingAgentSkill::new(Some("openai/gpt-4".to_string()));
        let m = skill.manifest();
        // model is private, verify indirectly via delegate output
        assert_eq!(m.name, "coding_agent");
    }

    #[test]
    fn test_new_without_model() {
        let skill = CodingAgentSkill::new(None);
        let m = skill.manifest();
        assert_eq!(m.name, "coding_agent");
    }

    // === manifest tests ===

    #[test]
    fn test_manifest_fields() {
        let skill = CodingAgentSkill::new(None);
        let m = skill.manifest();
        assert_eq!(m.name, "coding_agent");
        assert_eq!(m.version, "1.0.0");
        assert!(m.description.contains("AI coding agents"));
        assert_eq!(m.author, Some("CloseClaw Team".to_string()));
        assert!(m.dependencies.is_empty());
    }

    // === methods tests ===

    #[test]
    fn test_methods() {
        let skill = CodingAgentSkill::new(None);
        assert_eq!(
            skill.methods(),
            vec!["delegate", "review", "refactor", "test"]
        );
    }

    // === delegate tests ===

    #[tokio::test]
    async fn test_delegate_success() {
        let skill = CodingAgentSkill::new(Some("test/model".to_string()));
        let result = skill
            .execute(
                "delegate",
                serde_json::json!({"task": "write a function", "language": "python"}),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r["status"], "delegated");
        assert_eq!(r["task"], "write a function");
        assert_eq!(r["language"], "python");
        assert_eq!(r["model"], "test/model");
    }

    #[tokio::test]
    async fn test_delegate_default_language() {
        let skill = CodingAgentSkill::new(None);
        let result = skill
            .execute("delegate", serde_json::json!({"task": "fix bug"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["language"], "rust");
    }

    #[tokio::test]
    async fn test_delegate_missing_task() {
        let skill = CodingAgentSkill::new(None);
        let result = skill.execute("delegate", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("task")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    // === review tests ===

    #[tokio::test]
    async fn test_review_success() {
        let skill = CodingAgentSkill::new(None);
        let result = skill
            .execute("review", serde_json::json!({"code": "fn main() {}"}))
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r["status"], "review_complete");
        assert!(r["issues"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_review_missing_code() {
        let skill = CodingAgentSkill::new(None);
        let result = skill.execute("review", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("code")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    // === refactor tests ===

    #[tokio::test]
    async fn test_refactor_success() {
        let skill = CodingAgentSkill::new(None);
        let result = skill
            .execute(
                "refactor",
                serde_json::json!({"code": "fn old() {}", "goal": "reduce complexity"}),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r["status"], "refactored");
        assert_eq!(r["goal"], "reduce complexity");
    }

    #[tokio::test]
    async fn test_refactor_default_goal() {
        let skill = CodingAgentSkill::new(None);
        let result = skill
            .execute("refactor", serde_json::json!({"code": "fn old() {}"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["goal"], "improve readability");
    }

    #[tokio::test]
    async fn test_refactor_missing_code() {
        let skill = CodingAgentSkill::new(None);
        let result = skill.execute("refactor", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("code")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    // === test generation tests ===

    #[tokio::test]
    async fn test_test_success() {
        let skill = CodingAgentSkill::new(None);
        let result = skill
            .execute(
                "test",
                serde_json::json!({"code": "fn add(a:i32,b:i32)->i32{a+b}"}),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r["status"], "tests_generated");
        assert_eq!(r["test_count"], 0);
    }

    #[tokio::test]
    async fn test_test_missing_code() {
        let skill = CodingAgentSkill::new(None);
        let result = skill.execute("test", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("code")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    // === unknown method test ===

    #[tokio::test]
    async fn test_unknown_method() {
        let skill = CodingAgentSkill::new(None);
        let result = skill.execute("nonexistent", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::MethodNotFound {
                skill: s,
                method: m,
            } => {
                assert_eq!(s, "coding_agent");
                assert_eq!(m, "nonexistent");
            }
            other => panic!("expected MethodNotFound, got {:?}", other),
        }
    }
}
