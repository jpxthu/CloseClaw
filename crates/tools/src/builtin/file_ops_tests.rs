//! Unit tests for file_ops tools — metadata and permission-check tests.

use super::*;
use closeclaw_common::{PromptGenerationContext, WorkdirContext};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

pub(crate) fn make_cm() -> ConfigMgr {
    let tmp = TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

pub(crate) fn make_ctx(agent: &str) -> ToolContext {
    ToolContext {
        agent_id: agent.to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

// ---------------------------------------------------------------------------
// Metadata tests (migrated from inline)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_name_group_summary() {
    let tool = ReadTool::new(make_cm());
    assert_eq!(tool.name(), "Read");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
    assert!(!tool.flags().is_destructive);
}

#[tokio::test]
async fn test_write_name_group_summary() {
    let tool = WriteTool::new();
    assert_eq!(tool.name(), "Write");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_destructive);
    assert!(!tool.flags().is_read_only);
}

#[tokio::test]
async fn test_edit_name_group_summary() {
    let tool = EditTool::new();
    assert_eq!(tool.name(), "Edit");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_destructive);
    assert!(!tool.flags().is_read_only);
}

#[tokio::test]
async fn test_grep_name_group_summary() {
    let tool = GrepTool::new();
    assert_eq!(tool.name(), "Grep");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
}

#[tokio::test]
async fn test_ls_name_group_summary() {
    let tool = LsTool::new();
    assert_eq!(tool.name(), "Ls");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
}

#[tokio::test]
async fn test_read_input_schema_has_path() {
    let tool = ReadTool::new(make_cm());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
}

#[tokio::test]
async fn test_write_input_schema_has_path_and_content() {
    let tool = WriteTool::new();
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
    assert!(props.contains_key("content"));
}

#[tokio::test]
async fn test_grep_input_schema_has_pattern() {
    let tool = GrepTool::new();
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("pattern"));
}

#[tokio::test]
async fn test_ls_input_schema_optional_path() {
    let tool = LsTool::new();
    let schema = tool.input_schema();
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.is_empty());
}

// ---------------------------------------------------------------------------
// Permission tests — ReadTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("test.txt");
    std::fs::write(&file, "hello").unwrap();
    let tool = ReadTool::new(make_cm());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().data["content"], "hello\n");
}

// ---------------------------------------------------------------------------
// Permission tests — WriteTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("out.txt");
    let tool = WriteTool::new();
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "written"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "written");
}

// ---------------------------------------------------------------------------
// Permission tests — EditTool
// ---------------------------------------------------------------------------

// test_edit_allowed_with_rules removed — covered by test_edit_with_edits_array

// ---------------------------------------------------------------------------
// Permission tests — GrepTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_grep_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "target line").unwrap();
    let tool = GrepTool::new();
    let args = serde_json::json!({
        "pattern": "target",
        "path": tmp.path().to_str().unwrap()
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let results = result.unwrap().data["results"].as_array().unwrap().clone();
    assert!(!results.is_empty());
}

// ---------------------------------------------------------------------------
// Permission tests — LsTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ls_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("file.txt"), "").unwrap();
    let tool = LsTool::new();
    let args = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let tool_result = result.unwrap();
    let entries = tool_result.data["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e == "file.txt"));
}
// ---------------------------------------------------------------------------
// generate_prompt tests — ReadTool
// ---------------------------------------------------------------------------

/// `generate_prompt` must return context-aware output for empty context.
#[tokio::test]
async fn test_read_generate_prompt_empty_context() {
    let tool = ReadTool::new(make_cm());
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    // Empty context: no workdir, no combination suggestions
    assert!(!prompt.is_empty(), "prompt must not be empty");
    assert!(
        prompt.contains("Read"),
        "prompt should mention the Read tool name"
    );
    assert!(
        !prompt.contains("Working directory"),
        "empty context should not mention working directory"
    );
}

/// Workdir context changes the prompt output.
#[tokio::test]
async fn test_read_generate_prompt_includes_workdir() {
    let tool = ReadTool::new(make_cm());
    let no_workdir = PromptGenerationContext::default();
    let with_workdir = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/some/path".into(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        ..Default::default()
    };
    let prompt_no = tool.generate_prompt(&no_workdir);
    let prompt_yes = tool.generate_prompt(&with_workdir);
    assert_ne!(
        prompt_no, prompt_yes,
        "different workdir contexts should produce different prompts"
    );
    assert!(
        prompt_yes.contains("/some/path"),
        "prompt should contain the working directory path"
    );
    assert!(
        prompt_yes.contains("not a git repo"),
        "non-git path should note absence of git"
    );
    assert!(
        prompt_yes.contains("Relative paths"),
        "workdir guidance should mention relative path resolution"
    );
}

/// Git branch and recent_changes are reflected in the prompt.
#[tokio::test]
async fn test_read_generate_prompt_includes_git_info() {
    let tool = ReadTool::new(make_cm());
    let no_git = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/tmp".into(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        ..Default::default()
    };
    let with_git = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/tmp".into(),
            has_git: true,
            branch: Some("main".into()),
            recent_changes: 3,
        }),
        ..Default::default()
    };
    let prompt_no = tool.generate_prompt(&no_git);
    let prompt_yes = tool.generate_prompt(&with_git);
    assert!(
        prompt_yes.contains("main"),
        "git prompt should contain the branch name"
    );
    assert!(
        prompt_yes.contains("uncommitted change"),
        "git prompt should mention uncommitted changes"
    );
    assert!(
        prompt_no.contains("not a git repo"),
        "non-git prompt should note absence of git"
    );
}

/// Prompt includes combination suggestions when Write and Bash are available.
#[tokio::test]
async fn test_read_generate_prompt_combination_suggestions() {
    let tool = ReadTool::new(make_cm());
    let ctx = PromptGenerationContext {
        available_tool_names: vec!["Read".into(), "Write".into(), "Bash".into()],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("Write/Edit"),
        "should suggest Write/Edit as a combination"
    );
    assert!(
        prompt.contains("Bash"),
        "should suggest Bash as a combination"
    );
}

/// Full context produces a comprehensive prompt.
#[tokio::test]
async fn test_read_generate_prompt_full_context() {
    let tool = ReadTool::new(make_cm());
    let full_ctx = PromptGenerationContext {
        agent_id: "agent-1".into(),
        workdir: Some(WorkdirContext {
            path: "/home/user/project".into(),
            has_git: true,
            branch: Some("feat/x".into()),
            recent_changes: 7,
        }),
        available_tool_names: vec!["Read".into(), "Write".into(), "Bash".into()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let prompt = tool.generate_prompt(&full_ctx);
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("feat/x"));
    assert!(prompt.contains("uncommitted change"));
    assert!(prompt.contains("Combine with"));
}

// ---------------------------------------------------------------------------
// Edge cases — missing arguments
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_missing_path_arg() {
    let tool = ReadTool::new(make_cm());
    let result = tool.call(serde_json::json!({}), &make_ctx("a")).await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_write_missing_content_arg() {
    let tool = WriteTool::new();
    let result = tool
        .call(serde_json::json!({ "path": "/tmp/x" }), &make_ctx("a"))
        .await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_grep_missing_pattern_arg() {
    let tool = GrepTool::new();
    let result = tool.call(serde_json::json!({}), &make_ctx("a")).await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

/// WriteTool creates a new file when it doesn't exist.
#[tokio::test]
async fn test_write_new_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("newfile.txt");
    assert!(!path.exists());

    let tool = WriteTool::new();
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "created"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "created");
}

/// WriteTool overwrites an existing file.
#[tokio::test]
async fn test_write_overwrite_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("overwrite.txt");
    std::fs::write(&path, "original").unwrap();

    let tool = WriteTool::new();
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "replaced"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "replaced");
}

// ---------------------------------------------------------------------------
// Step 1.6: Image integration tests — offset/limit and runtime flags.
// ---------------------------------------------------------------------------

/// ReadTool declares `is_read_only: true` and `is_concurrency_safe: true`
/// per the design doc ("Read 工具标记为只读工具和并发安全工具").
#[tokio::test]
async fn test_read_flags_read_only_and_concurrency_safe() {
    let tool = ReadTool::new(make_cm());
    let flags = tool.flags();
    assert!(flags.is_read_only, "ReadTool must be read-only");
    assert!(
        flags.is_concurrency_safe,
        "ReadTool must be concurrency-safe"
    );
}

/// Helper: create a temporary image file and verify that offset/limit
/// parameters do not affect the output — images always return in full.
async fn assert_image_ignores_offset_limit(
    ext: &str,
    format: image::ImageFormat,
    create_fn: impl FnOnce() -> image::DynamicImage,
) {
    let tmp = TempDir::new().unwrap();
    let img = create_fn();
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, format).unwrap();
    let raw = buf.into_inner();
    let path = tmp.path().join(format!("photo.{ext}"));
    std::fs::write(&path, &raw).unwrap();
    let tool = ReadTool::new(make_cm());
    let args = serde_json::json!({ "path": path.to_str().unwrap(), "offset": 50, "limit": 2 });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = result.unwrap().data["content"]
        .as_str()
        .unwrap()
        .to_string();
    let args_no = serde_json::json!({ "path": path.to_str().unwrap() });
    let content_no = tool.call(args_no, &make_ctx("a")).await.unwrap().data["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(content, content_no);
    assert!(content.starts_with("[Image file:"));
    assert!(content.contains("data:image/png;base64,"));
}

#[tokio::test]
async fn test_read_image_ignores_offset_limit() {
    // PNG (Rgba).
    assert_image_ignores_offset_limit("png", image::ImageFormat::Png, || {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(100, 50, |_, _| {
            image::Rgba([255, 0, 0, 255])
        }))
    })
    .await;
    // JPEG (Rgb — JPEG doesn't support Rgba).
    assert_image_ignores_offset_limit("jpg", image::ImageFormat::Jpeg, || {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(80, 60, |_, _| {
            image::Rgb([0, 0, 255])
        }))
    })
    .await;
}
