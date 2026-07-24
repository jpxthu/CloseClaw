//! Built-in tools — skill creator.
//!
//! Creates and validates agent skill files (SKILL.md with frontmatter).

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

// ---------------------------------------------------------------------------
// SkillCreatorTool
// ---------------------------------------------------------------------------

/// Tool for authoring and validating agent skill files.
pub struct SkillCreatorTool;

impl Default for SkillCreatorTool {
    fn default() -> Self {
        Self
    }
}

impl SkillCreatorTool {
    pub fn new() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolCallError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolCallError::InvalidArgs(format!("missing required parameter: {key}")))
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Default skills directory: `{cwd}/.closeclaw/skills/`
fn default_skills_dir(ctx: &ToolContext) -> PathBuf {
    let cwd = ctx
        .workdir
        .as_ref()
        .map(|w| PathBuf::from(&w.path))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    cwd.join(".closeclaw").join("skills")
}

/// Generate SKILL.md content from parameters.
fn build_skill_md(description: &str, body: &str) -> String {
    let escaped = description.replace('"', "\\\"").replace('\n', " ");
    let mut content = format!("---\ndescription: \"{escaped}\"\n---\n");
    if !body.is_empty() {
        content.push('\n');
        content.push_str(body);
        if !body.ends_with('\n') {
            content.push('\n');
        }
    }
    content
}

/// Validate SKILL.md content format.
///
/// Returns Ok(()) if valid, or Err with reason.
fn validate_skill_md(content: &str) -> Result<(), String> {
    let trimmed = content.trim_start();
    if !(trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n")) {
        return Err(
            "missing frontmatter (content must start with `---` followed by a newline)".into(),
        );
    }
    let after_first = &trimmed[3..].trim_start_matches('\r');
    let after_first = &after_first[1..]; // skip the newline
    let end = after_first
        .find("---")
        .ok_or_else(|| "unclosed frontmatter (missing closing `---`)".to_string())?;
    let frontmatter = &after_first[..end];
    if !frontmatter.contains("description") {
        return Err("missing required field `description` in frontmatter".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tool impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for SkillCreatorTool {
    fn name(&self) -> &str {
        "SkillCreator"
    }

    fn group(&self) -> &str {
        "skill_creator"
    }

    fn summary(&self) -> String {
        "Create or validate agent skills".to_string()
    }

    fn detail(&self) -> String {
        "Creates new agent skill files (SKILL.md) or validates existing ones. \
         Use when creating a new skill from scratch or when asked to validate \
         a skill file's format."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        let skills_dir_desc = concat!(
            "Target skills directory ",
            "(create only, optional). ",
            "Defaults to .closeclaw/skills/ ",
            "under cwd"
        );
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: create or validate",
                    "enum": ["create", "validate"]
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (used as directory name)"
                },
                "description": {
                    "type": "string",
                    "description": "Natural language description of the skill purpose"
                },
                "body": {
                    "type": "string",
                    "description": "Instruction body text for the skill (create only, optional)"
                },
                "skills_dir": {
                    "type": "string",
                    "description": skills_dir_desc,
                },
                "content": {
                    "type": "string",
                    "description": "SKILL.md content text to validate (validate only)"
                }
            },
            "required": ["action"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_destructive: true,
            is_deferred_by_default: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let action = required_str(&args, "action")?;
        match action {
            "create" => self.handle_create(&args, ctx),
            "validate" => self.handle_validate(&args),
            other => Err(ToolCallError::InvalidArgs(format!(
                "unknown action: {other} (expected \"create\" or \"validate\")"
            ))),
        }
    }
}

impl SkillCreatorTool {
    /// Handle `create` action: create skill directory and SKILL.md.
    fn handle_create(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let name = required_str(args, "name")?;
        let description = required_str(args, "description")?;
        let body = optional_str(args, "body").unwrap_or("");
        let skills_dir = optional_str(args, "skills_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_skills_dir(ctx));

        // Validate name: only alphanumeric, hyphens, underscores
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ToolCallError::InvalidArgs(format!(
                "invalid skill name: {name} (only alphanumeric, hyphens, underscores allowed)"
            )));
        }

        let skill_dir = skills_dir.join(name);
        let skill_file = skill_dir.join("SKILL.md");

        // Create directory
        std::fs::create_dir_all(&skill_dir).map_err(|e| {
            ToolCallError::ExecutionFailed(format!("failed to create directory: {e}"))
        })?;

        // Build and write SKILL.md
        let content = build_skill_md(description, body);
        std::fs::write(&skill_file, &content).map_err(|e| {
            ToolCallError::ExecutionFailed(format!("failed to write SKILL.md: {e}"))
        })?;

        Ok(ToolResult {
            data: serde_json::json!({
                "status": "created",
                "path": skill_file.to_string_lossy(),
                "name": name,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }

    /// Handle `validate` action: validate SKILL.md content format.
    fn handle_validate(&self, args: &Value) -> Result<ToolResult, ToolCallError> {
        let content = required_str(args, "content")?;

        match validate_skill_md(content) {
            Ok(()) => Ok(ToolResult {
                data: serde_json::json!({
                    "valid": true,
                }),
                new_messages: vec![],
                context_modifier: None,
            }),
            Err(reason) => Ok(ToolResult {
                data: serde_json::json!({
                    "valid": false,
                    "reason": reason,
                }),
                new_messages: vec![],
                context_modifier: None,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_creator_name() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.name(), "SkillCreator");
    }

    #[test]
    fn test_skill_creator_group() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.group(), "skill_creator");
    }

    #[test]
    fn test_skill_creator_summary_len() {
        let tool = SkillCreatorTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_skill_creator_flags_deferred() {
        let tool = SkillCreatorTool::new();
        assert!(tool.flags().is_deferred_by_default);
    }

    #[test]
    fn test_skill_creator_flags_destructive() {
        let tool = SkillCreatorTool::new();
        assert!(tool.flags().is_destructive);
    }

    #[test]
    fn test_skill_creator_schema_has_required_fields() {
        let tool = SkillCreatorTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("action"));
        assert!(props.contains_key("name"));
        assert!(props.contains_key("description"));
        assert!(props.contains_key("body"));
        assert!(props.contains_key("skills_dir"));
        assert!(props.contains_key("content"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("action")));
    }

    #[test]
    fn test_build_skill_md() {
        let md = build_skill_md("A test skill", "Do something.");
        assert!(md.starts_with("---\ndescription: \"A test skill\"\n---\n"));
        assert!(md.contains("Do something."));
    }

    #[test]
    fn test_build_skill_md_empty_body() {
        let md = build_skill_md("Desc only", "");
        assert_eq!(md, "---\ndescription: \"Desc only\"\n---\n");
    }

    #[test]
    fn test_build_skill_md_escapes_quotes() {
        let md = build_skill_md("He said \"hello\"", "");
        assert!(md.contains("description: \"He said \\\"hello\\\"\""));
        assert!(validate_skill_md(&md).is_ok());
    }

    #[test]
    fn test_validate_skill_md_valid() {
        let content = "---\ndescription: \"Hello\"\n---\n\nSome instructions.";
        assert!(validate_skill_md(content).is_ok());
    }

    #[test]
    fn test_validate_skill_md_no_frontmatter() {
        let content = "Just some text without frontmatter";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("missing frontmatter"));
    }

    #[test]
    fn test_validate_skill_md_no_description() {
        let content = "---\ntitle: \"My Skill\"\n---\nBody here.";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("missing required field `description`"));
    }

    #[test]
    fn test_validate_skill_md_unclosed_frontmatter() {
        let content = "---\ndescription: \"test\"";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("unclosed frontmatter"));
    }

    #[test]
    fn test_validate_skill_md_pure_text() {
        let content = "This is just plain text without any frontmatter markers";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("missing frontmatter"));
    }

    #[test]
    fn test_default_skills_dir() {
        let ctx = ToolContext {
            agent_id: "test".into(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let dir = default_skills_dir(&ctx);
        assert!(dir.to_string_lossy().contains(".closeclaw/skills"));
    }

    #[test]
    fn test_optional_str() {
        let args = serde_json::json!({ "a": "hello", "b": "" });
        assert_eq!(optional_str(&args, "a"), Some("hello"));
        assert_eq!(optional_str(&args, "b"), None);
        assert_eq!(optional_str(&args, "missing"), None);
    }

    // ------------------------------------------------------------------
    // call() integration tests
    // ------------------------------------------------------------------

    fn make_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        }
    }

    fn make_ctx_with_workdir(path: &std::path::Path) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            workdir: Some(crate::WorkdirContext {
                path: path.to_string_lossy().into(),
                has_git: false,
                branch: None,
                recent_changes: 0,
            }),
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        }
    }

    #[tokio::test]
    async fn test_call_create_normal() {
        let tool = SkillCreatorTool::new();
        let temp = tempfile::tempdir().unwrap();
        let ctx = make_ctx_with_workdir(temp.path());
        let args = serde_json::json!({
            "action": "create",
            "name": "my-skill",
            "description": "A test skill",
            "body": "# Instructions\nDo things.",
        });
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["status"], "created");
        assert_eq!(result.data["name"], "my-skill");
        let path = std::path::PathBuf::from(result.data["path"].as_str().unwrap());
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("description: \"A test skill\""));
        assert!(content.contains("# Instructions"));
    }

    #[tokio::test]
    async fn test_call_create_duplicate_name() {
        let tool = SkillCreatorTool::new();
        let temp = tempfile::tempdir().unwrap();
        let ctx = make_ctx_with_workdir(temp.path());
        let args = serde_json::json!({
            "action": "create",
            "name": "dup-skill",
            "description": "First",
        });
        // First call succeeds
        let _ = tool.call(args.clone(), &ctx).await.unwrap();
        // Second call with same name succeeds (overwrites)
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["status"], "created");
    }

    #[tokio::test]
    async fn test_call_create_missing_name() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "create",
            "description": "Missing name",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::InvalidArgs(msg) => {
                assert!(msg.contains("missing required parameter: name"))
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_create_missing_description() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "create",
            "name": "no-desc",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::InvalidArgs(msg) => {
                assert!(msg.contains("missing required parameter: description"))
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_create_invalid_name_chars() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "create",
            "name": "has space!",
            "description": "Invalid name",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::InvalidArgs(msg) => assert!(msg.contains("invalid skill name")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_create_invalid_directory() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "create",
            "name": "fail-skill",
            "description": "Will fail",
            "skills_dir": "/nonexistent/deeply/nested/path",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::ExecutionFailed(msg) => {
                assert!(msg.contains("failed to create directory"))
            }
            other => panic!("expected ExecutionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_validate_valid() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "validate",
            "content": "---\ndescription: \"Hello\"\n---\n\nDo stuff.",
        });
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["valid"], true);
    }

    #[tokio::test]
    async fn test_call_validate_no_frontmatter() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "validate",
            "content": "Just plain text",
        });
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["valid"], false);
        assert!(result.data["reason"]
            .as_str()
            .unwrap()
            .contains("missing frontmatter"));
    }

    #[tokio::test]
    async fn test_call_validate_missing_description() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "validate",
            "content": "---\ntitle: \"No desc\"\n---\nBody.",
        });
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["valid"], false);
        assert!(result.data["reason"]
            .as_str()
            .unwrap()
            .contains("missing required field `description`"));
    }

    #[tokio::test]
    async fn test_call_validate_pure_text() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "validate",
            "content": "This is just plain text without any frontmatter markers",
        });
        let result = tool.call(args, &ctx).await.unwrap();
        assert_eq!(result.data["valid"], false);
    }

    #[tokio::test]
    async fn test_call_validate_missing_content() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "validate",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::InvalidArgs(msg) => {
                assert!(msg.contains("missing required parameter: content"))
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_unknown_action() {
        let tool = SkillCreatorTool::new();
        let ctx = make_ctx();
        let args = serde_json::json!({
            "action": "delete",
        });
        let err = tool.call(args, &ctx).await.unwrap_err();
        match err {
            ToolCallError::InvalidArgs(msg) => assert!(msg.contains("unknown action")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[test]
    fn test_input_schema_valid_json() {
        let tool = SkillCreatorTool::new();
        let schema = tool.input_schema();
        // Schema should be a valid JSON object
        assert!(schema.is_object());
        // Should have type: object
        assert_eq!(schema["type"], "object");
        // Should have properties
        assert!(schema.pointer("/properties").is_some());
        // Should have required array with "action"
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("action")));
        // All expected properties should be present
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("action"));
        assert!(props.contains_key("name"));
        assert!(props.contains_key("description"));
        assert!(props.contains_key("body"));
        assert!(props.contains_key("skills_dir"));
        assert!(props.contains_key("content"));
    }

    // ------------------------------------------------------------------
    // Step 1.5: validate_skill_md strictness tests
    // ------------------------------------------------------------------

    #[test]
    fn test_validate_rejects_embedded_dashes() {
        // "a---b---c" starts with 'a', not '---'
        let content = "a---b---c\ndescription: \"test\"\n---\n";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("missing frontmatter"));
    }

    #[test]
    fn test_validate_rejects_no_newline_after_dashes() {
        // "--- description" has space, not newline after ---
        let content = "--- description\n---\n";
        let err = validate_skill_md(content).unwrap_err();
        assert!(err.contains("missing frontmatter"));
    }

    #[test]
    fn test_validate_accepts_crlf() {
        let content = "---\r\ndescription: \"test\"\r\n---\r\n";
        assert!(validate_skill_md(content).is_ok());
    }

    #[test]
    fn test_validate_accepts_lf() {
        let content = "---\ndescription: \"test\"\n---\n";
        assert!(validate_skill_md(content).is_ok());
    }

    #[test]
    fn test_validate_rejects_empty() {
        let err = validate_skill_md("").unwrap_err();
        assert!(err.contains("missing frontmatter"));
    }

    #[test]
    fn test_build_skill_md_escapes_newlines() {
        let md = build_skill_md("line1\nline2", "body");
        assert!(md.contains("description: \"line1 line2\""));
        assert!(!md.contains("line1\nline2"));
    }

    #[test]
    fn test_build_skill_md_escapes_mixed() {
        let md = build_skill_md("say \"hi\"\nand newline", "");
        assert!(md.contains("description: \"say \\\"hi\\\" and newline\""));
    }
}
