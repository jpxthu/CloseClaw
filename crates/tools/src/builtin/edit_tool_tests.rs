//! EditTool-specific integration tests — permissions, schema, edits array,
//! replace_all, error paths, and edits_applied return value.

use crate::builtin::file_ops::EditTool;
use crate::{Tool, ToolCallError};
use tempfile::TempDir;

use super::file_ops::tests::{
    allow_file, allow_tool, make_af, make_af_deny, make_cm, make_ctx, make_engine, make_sm,
};

fn make_edit_tool(rules: Vec<closeclaw_permission::engine::engine_types::Rule>) -> EditTool {
    EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af())
}

// ---------------------------------------------------------------------------
// Permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("edit.txt");
    std::fs::write(&path, "original").unwrap();
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "edits": [{ "oldText": "original", "newText": "changed" }]
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "original");
}

// ---------------------------------------------------------------------------
// Schema tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_input_schema_has_all_fields() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
    assert!(props.contains_key("edits"));
    assert!(props.contains_key("replace_all"));
    // Legacy top-level oldText/newText must NOT be exposed.
    assert!(!props.contains_key("oldText"));
    assert!(!props.contains_key("newText"));
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.contains(&serde_json::json!("path")));
    assert!(required.contains(&serde_json::json!("edits")));
}

// ---------------------------------------------------------------------------
// Edits array tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_with_edits_array() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("multi.txt");
    std::fs::write(&path, "aaa bbb ccc").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "edits": [
            { "oldText": "aaa", "newText": "AAA" },
            { "oldText": "ccc", "newText": "CCC" }
        ]
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "AAA bbb CCC");
}

#[tokio::test]
async fn test_edit_legacy_old_new_text_rejected() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("legacy.txt");
    std::fs::write(&path, "before middle after").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let args =
        serde_json::json!({"path":path.to_str().unwrap(),"oldText":"middle","newText":"CENTER"});
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

// ---------------------------------------------------------------------------
// replace_all tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_replace_all() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("replace_all.txt");
    std::fs::write(&path, "foo bar foo baz foo").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "edits": [
            { "oldText": "foo", "newText": "FOO" }
        ],
        "replace_all": true
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "FOO bar FOO baz FOO");
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_old_text_not_found() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("notfound.txt");
    std::fs::write(&path, "hello world").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let args = serde_json::json!({"path":path.to_str().unwrap(),"edits":[{"oldText":"nonexistent","newText":"replacement"}]});
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::ExecutionFailed(msg) => assert!(msg.contains("not found")),
        other => panic!("expected ExecutionFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_edit_empty_edits_array_rejected() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({"path":"/tmp/x","edits":[]});
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_edit_missing_edits_arg() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let result = tool
        .call(serde_json::json!({ "path": "/tmp/x" }), &make_ctx("a"))
        .await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

// ---------------------------------------------------------------------------
// edits_applied return value — single edit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_returns_edits_applied_single() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("single.txt");
    std::fs::write(&path, "alpha beta").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let r = tool
        .call(
            serde_json::json!({"path":path,"edits":[{"oldText":"alpha","newText":"ALPHA"}]}),
            &make_ctx("a"),
        )
        .await
        .unwrap();
    assert_eq!(r.data["edits_applied"], 1);
    assert_eq!(r.data["content"], "ALPHA beta");
}

// ---------------------------------------------------------------------------
// edits_applied return value — multiple edits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_returns_edits_applied_multi() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("multi.txt");
    std::fs::write(&path, "aaa bbb ccc").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let r = tool
        .call(
            serde_json::json!({
                "path":path,
                "edits":[{"oldText":"aaa","newText":"AAA"},{"oldText":"ccc","newText":"CCC"}]
            }),
            &make_ctx("a"),
        )
        .await
        .unwrap();
    assert_eq!(r.data["edits_applied"], 2);
}

// ---------------------------------------------------------------------------
// edits_applied return value — replace_all
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_returns_edits_applied_replace_all() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("ra.txt");
    std::fs::write(&path, "foo bar foo baz foo").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = make_edit_tool(rules);
    let r = tool
        .call(
            serde_json::json!({"path":path,"edits":[{"oldText":"foo","newText":"FOO"}],"replace_all":true}),
            &make_ctx("a"),
        )
        .await
        .unwrap();
    assert_eq!(r.data["edits_applied"], 1);
    assert_eq!(r.data["content"], "FOO bar FOO baz FOO");
}
