//! Unit tests for skill_creator tool.

use super::*;
use crate::builtin::skill_creator::{
    build_skill_md, default_skills_dir, edit_skill_md, optional_str, validate_skill_md,
};
use crate::{Tool, ToolCallError, ToolContext};

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
    assert!(props.contains_key("path"));
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
    let content = "This is just plain text without any frontmatter \
                   markers";
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
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("invalid skill name"))
        }
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
        "content": "This is just plain text without any frontmatter \
                    markers",
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
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("unknown action"))
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[test]
fn test_input_schema_valid_json() {
    let tool = SkillCreatorTool::new();
    let schema = tool.input_schema();
    assert!(schema.is_object());
    assert_eq!(schema["type"], "object");
    assert!(schema.pointer("/properties").is_some());
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.contains(&serde_json::json!("action")));
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("action"));
    assert!(props.contains_key("name"));
    assert!(props.contains_key("description"));
    assert!(props.contains_key("body"));
    assert!(props.contains_key("skills_dir"));
    assert!(props.contains_key("content"));
    assert!(props.contains_key("path"));
}

// ------------------------------------------------------------------
// Step 1.5: validate_skill_md strictness tests
// ------------------------------------------------------------------

#[test]
fn test_validate_rejects_embedded_dashes() {
    let content = "a---b---c\ndescription: \"test\"\n---\n";
    let err = validate_skill_md(content).unwrap_err();
    assert!(err.contains("missing frontmatter"));
}

#[test]
fn test_validate_rejects_no_newline_after_dashes() {
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

// ------------------------------------------------------------------
// Step 1.6: edit_skill_md tests
// ------------------------------------------------------------------

#[test]
fn test_edit_skill_md_update_description() {
    let input = "---\ndescription: \"old\"\n---\n\nBody.\n";
    let out = edit_skill_md(input, Some("new desc"), None);
    assert!(out.contains("description: \"new desc\""));
    assert!(out.contains("Body."));
    assert!(validate_skill_md(&out).is_ok());
}

#[test]
fn test_edit_skill_md_update_body() {
    let input = "---\ndescription: \"keep\"\n---\n\nOld body.\n";
    let out = edit_skill_md(input, None, Some("New body."));
    assert!(out.contains("description: \"keep\""));
    assert!(out.contains("New body."));
    assert!(!out.contains("Old body."));
}

#[test]
fn test_edit_skill_md_update_both() {
    let input = "---\ndescription: \"old\"\n---\n\nOld body.\n";
    let out = edit_skill_md(input, Some("new"), Some("new body"));
    assert!(out.contains("description: \"new\""));
    assert!(out.contains("new body"));
    assert!(!out.contains("old"));
    assert!(!out.contains("Old body."));
}

#[test]
fn test_edit_skill_md_no_body_no_trailing_newline() {
    let input = "---\ndescription: \"test\"\n---\n";
    let out = edit_skill_md(input, Some("updated"), None);
    assert!(out.contains("description: \"updated\""));
    assert!(validate_skill_md(&out).is_ok());
}

#[test]
fn test_edit_skill_md_preserves_extra_frontmatter() {
    let input = "---\ndescription: \"d\"\nwhen-to-use: \"always\"\n---\n\nBody.\n";
    let out = edit_skill_md(input, Some("new d"), None);
    assert!(out.contains("description: \"new d\""));
    assert!(out.contains("when-to-use: \"always\""));
}

#[test]
fn test_edit_skill_md_escapes_quotes_in_desc() {
    let input = "---\ndescription: \"old\"\n---\n\nBody.\n";
    let out = edit_skill_md(input, Some("say \"hi\""), None);
    assert!(out.contains("description: \"say \\\"hi\\\"\""));
}

#[test]
fn test_edit_skill_md_escapes_newlines_in_desc() {
    let input = "---\ndescription: \"old\"\n---\n\nBody.\n";
    let out = edit_skill_md(input, Some("line1\nline2"), None);
    assert!(out.contains("description: \"line1 line2\""));
}

#[test]
fn test_edit_skill_md_crlf() {
    let input = "---\r\ndescription: \"old\"\r\n---\r\n\r\nBody.\r\n";
    let out = edit_skill_md(input, Some("new"), None);
    assert!(out.contains("description: \"new\""));
    assert!(validate_skill_md(&out).is_ok());
}

#[test]
fn test_edit_skill_md_pure_crlf_no_trim() {
    // Pure CRLF input WITHOUT trim_start — exercises fm_start fix
    let input = "---\r\ndescription: \"old\"\r\n---\r\n\r\nBody.\r\n";
    let out = edit_skill_md(input, Some("new"), None);
    assert!(out.contains("description: \"new\""));
    assert!(out.contains("Body."));
    // No stray leading newline in body
    assert!(!out.starts_with("\n"));
    assert!(validate_skill_md(&out).is_ok());
}

// ------------------------------------------------------------------
// Step 1.6: call() edit integration tests
// ------------------------------------------------------------------

#[tokio::test]
async fn test_call_edit_description() {
    let tool = SkillCreatorTool::new();
    let temp = tempfile::tempdir().unwrap();
    let skill_file = temp.path().join("SKILL.md");
    std::fs::write(&skill_file, "---\ndescription: \"old\"\n---\n\nBody.\n").unwrap();
    let ctx = make_ctx_with_workdir(temp.path());
    let args = serde_json::json!({
        "action": "edit",
        "path": skill_file.to_string_lossy(),
        "description": "new desc",
    });
    let result = tool.call(args, &ctx).await.unwrap();
    assert_eq!(result.data["status"], "edited");
    let content = std::fs::read_to_string(&skill_file).unwrap();
    assert!(content.contains("description: \"new desc\""));
    assert!(content.contains("Body."));
}

#[tokio::test]
async fn test_call_edit_body() {
    let tool = SkillCreatorTool::new();
    let temp = tempfile::tempdir().unwrap();
    let skill_file = temp.path().join("SKILL.md");
    std::fs::write(
        &skill_file,
        "---\ndescription: \"keep\"\n---\n\nOld body.\n",
    )
    .unwrap();
    let ctx = make_ctx_with_workdir(temp.path());
    let args = serde_json::json!({
        "action": "edit",
        "path": skill_file.to_string_lossy(),
        "body": "New body.",
    });
    let result = tool.call(args, &ctx).await.unwrap();
    assert_eq!(result.data["status"], "edited");
    let content = std::fs::read_to_string(&skill_file).unwrap();
    assert!(content.contains("description: \"keep\""));
    assert!(content.contains("New body."));
    assert!(!content.contains("Old body."));
}

#[tokio::test]
async fn test_call_edit_nothing_provided() {
    let tool = SkillCreatorTool::new();
    let ctx = make_ctx();
    let args = serde_json::json!({
        "action": "edit",
        "path": "/tmp/x",
    });
    let err = tool.call(args, &ctx).await.unwrap_err();
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("at least one of"))
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_call_edit_missing_path() {
    let tool = SkillCreatorTool::new();
    let ctx = make_ctx();
    let args = serde_json::json!({
        "action": "edit",
        "description": "new",
    });
    let err = tool.call(args, &ctx).await.unwrap_err();
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("missing required parameter: path"))
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_call_edit_file_not_found() {
    let tool = SkillCreatorTool::new();
    let ctx = make_ctx();
    let args = serde_json::json!({
        "action": "edit",
        "path": "/nonexistent/SKILL.md",
        "description": "new",
    });
    let err = tool.call(args, &ctx).await.unwrap_err();
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("file not found"))
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_call_edit_invalid_format() {
    let tool = SkillCreatorTool::new();
    let temp = tempfile::tempdir().unwrap();
    let skill_file = temp.path().join("SKILL.md");
    std::fs::write(&skill_file, "no frontmatter here").unwrap();
    let ctx = make_ctx_with_workdir(temp.path());
    let args = serde_json::json!({
        "action": "edit",
        "path": skill_file.to_string_lossy(),
        "description": "new",
    });
    let err = tool.call(args, &ctx).await.unwrap_err();
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("invalid SKILL.md format"))
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_call_edit_preserves_extra_frontmatter() {
    let tool = SkillCreatorTool::new();
    let temp = tempfile::tempdir().unwrap();
    let skill_file = temp.path().join("SKILL.md");
    let original = "---\ndescription: \"d\"\n\
                    when-to-use: \"always\"\n---\n\nBody.\n";
    std::fs::write(&skill_file, original).unwrap();
    let ctx = make_ctx_with_workdir(temp.path());
    let args = serde_json::json!({
        "action": "edit",
        "path": skill_file.to_string_lossy(),
        "description": "new d",
    });
    let result = tool.call(args, &ctx).await.unwrap();
    assert_eq!(result.data["status"], "edited");
    let content = std::fs::read_to_string(&skill_file).unwrap();
    assert!(content.contains("description: \"new d\""));
    assert!(content.contains("when-to-use: \"always\""));
}
