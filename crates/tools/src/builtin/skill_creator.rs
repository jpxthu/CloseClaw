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

pub(crate) fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolCallError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolCallError::InvalidArgs(format!("missing required parameter: {key}")))
}

pub(crate) fn optional_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Default skills directory: `{cwd}/.closeclaw/skills/`
pub(crate) fn default_skills_dir(ctx: &ToolContext) -> PathBuf {
    let cwd = ctx
        .workdir
        .as_ref()
        .map(|w| PathBuf::from(&w.path))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    cwd.join(".closeclaw").join("skills")
}

/// Generate SKILL.md content from parameters.
pub(crate) fn build_skill_md(description: &str, body: &str) -> String {
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
pub(crate) fn validate_skill_md(content: &str) -> Result<(), String> {
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

/// Modify an existing SKILL.md file content.
///
/// Replaces the `description` field in frontmatter and/or the body
/// text after the closing `---`. Preserves all other frontmatter
/// fields unchanged.
pub(crate) fn edit_skill_md(
    content: &str,
    new_description: Option<&str>,
    new_body: Option<&str>,
) -> String {
    let trimmed = content.trim_start();
    let leading_len = content.len() - trimmed.len();
    let prefix = &content[..leading_len];

    let fm_start = if trimmed.starts_with("---\r\n") { 5 } else { 4 };
    let closing_marker = trimmed[fm_start..].find("---").unwrap();
    let fm_end = fm_start + closing_marker;
    let fm_content = &trimmed[fm_start..fm_end];

    let fm_content = if let Some(desc) = new_description {
        let escaped = desc.replace('"', "\\\"").replace('\n', " ");
        if let Some(start) = fm_content.find("description:") {
            let before = &fm_content[..start];
            let after_desc = &fm_content[start..];
            let line_end = after_desc.find('\n').unwrap_or(after_desc.len());
            let after = &after_desc[line_end..];
            format!("{before}description: \"{escaped}\"{after}")
        } else {
            format!("description: \"{escaped}\"\n{fm_content}")
        }
    } else {
        fm_content.to_string()
    };

    let after_fm = &trimmed[fm_end..];
    let after_fm = if after_fm.starts_with("\r\n") {
        &after_fm[2..]
    } else if after_fm.starts_with('\n') {
        &after_fm[1..]
    } else {
        after_fm
    };

    let body = if let Some(b) = new_body {
        b.to_string()
    } else {
        after_fm.to_string()
    };

    let mut out = String::new();
    out.push_str(prefix);
    out.push_str("---\n");
    out.push_str(&fm_content);
    out.push_str("---\n");
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    out
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
                    "description": "Action to perform: create, validate, or edit",
                    "enum": ["create", "validate", "edit"]
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
                },
                "path": {
                    "type": "string",
                    "description": "Absolute path to SKILL.md file (edit only)"
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
            "edit" => self.handle_edit(&args),
            other => Err(ToolCallError::InvalidArgs(format!(
                "unknown action: {other} \
                  (expected \"create\", \"validate\", or \"edit\")"
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

    /// Handle `edit` action: modify an existing SKILL.md file.
    fn handle_edit(&self, args: &Value) -> Result<ToolResult, ToolCallError> {
        let path = required_str(args, "path")?;
        let description = optional_str(args, "description");
        let body = optional_str(args, "body");

        if description.is_none() && body.is_none() {
            return Err(ToolCallError::InvalidArgs(
                "at least one of \"description\" or \
                  \"body\" must be provided"
                    .into(),
            ));
        }

        let file_path = PathBuf::from(path);
        if !file_path.exists() {
            return Err(ToolCallError::InvalidArgs(format!(
                "file not found: {path}"
            )));
        }

        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| ToolCallError::ExecutionFailed(format!("failed to read file: {e}")))?;

        validate_skill_md(&content)
            .map_err(|e| ToolCallError::InvalidArgs(format!("invalid SKILL.md format: {e}")))?;

        let modified = edit_skill_md(&content, description, body);

        std::fs::write(&file_path, &modified)
            .map_err(|e| ToolCallError::ExecutionFailed(format!("failed to write file: {e}")))?;

        Ok(ToolResult {
            data: serde_json::json!({
                "status": "edited",
                "path": path,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
