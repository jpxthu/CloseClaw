//! Coding Agent Skill - Delegate coding tasks to AI coding agents
//!
//! This skill wraps OpenCode or Claude Code to handle complex
//! coding tasks.

use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;

pub struct CodingAgentSkill;

impl CodingAgentSkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// task is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "coding_agent",
            "description": "Delegate complex coding tasks to AI coding agents",
            "supported_actions": ["delegate"],
            "usage": {
                "delegate": {"task": "<task_description>"}
            }
        })
        .to_string()
    }

    /// Build structured delegation parameters for a coding task.
    fn build_delegation_params(task: &str) -> String {
        json!({
            "skill": "coding_agent",
            "action": "delegate",
            "task": task,
            "guidance": "Delegate the coding task to an AI agent",
            "agents": ["opencode", "claude-code"]
        })
        .to_string()
    }
}

impl Default for CodingAgentSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for CodingAgentSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "coding_agent".to_string(),
            version: "1.0.0".to_string(),
            description: "Delegate complex coding tasks to AI coding agents \
                    (OpenCode, Claude Code)"
                .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Coding Agent Skill

Use the `exec` tool to delegate coding tasks to an AI coding agent.

- **Delegate**: `exec` with `opencode run "<task>"` or `claude-code "<task>"`
- **Review code**: Read the file first with `read`, then ask for review.
- **Refactor**: Read the file, then delegate the refactoring task.
- **Generate tests**: Read the source file, then delegate test generation.

Always read relevant files before delegating to provide context."#
    }

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        match args.get("task").and_then(|v| v.as_str()) {
            None => Ok(Self::capabilities_description()),
            Some(t) => Ok(Self::build_delegation_params(t)),
        }
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to delegate \
                complex coding tasks to an AI coding agent like \
                OpenCode or Claude Code"
                .to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Medium,
        }
    }
}
