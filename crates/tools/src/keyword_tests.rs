//! Unit tests for keyword extraction and build_descriptor keyword integration.

use super::*;
use crate::ToolFlags;
use closeclaw_common::tool_registry::ToolRegistryQuery;

// =========================================================================
// extract_keywords — keyword extraction from detail strings
// =========================================================================

#[test]
fn test_extract_keywords_normal_input() {
    let detail = "[keywords: read file cat view] some detail here";
    let kw = extract_keywords(detail);
    assert_eq!(kw, vec!["read", "file", "cat", "view"]);
}

#[test]
fn test_extract_keywords_single_keyword() {
    let detail = "[keywords: read] the file content";
    let kw = extract_keywords(detail);
    assert_eq!(kw, vec!["read"]);
}

#[test]
fn test_extract_keywords_no_prefix_returns_empty() {
    let detail = "Just a regular description without keywords";
    let kw = extract_keywords(detail);
    assert!(kw.is_empty(), "expected empty vec, got {:?}", kw);
}

#[test]
fn test_extract_keywords_empty_string() {
    let kw = extract_keywords("");
    assert!(kw.is_empty());
}

#[test]
fn test_extract_keywords_prefix_in_middle_not_matched() {
    let detail = "Some text before [keywords: read file] and after";
    let kw = extract_keywords(detail);
    assert!(
        kw.is_empty(),
        "prefix in middle should not match, got {:?}",
        kw
    );
}

#[test]
fn test_extract_keywords_empty_keywords_list() {
    let detail = "[keywords: ] some detail";
    let kw = extract_keywords(detail);
    assert!(
        kw.is_empty(),
        "empty keywords should return empty vec, got {:?}",
        kw
    );
}

#[test]
fn test_extract_keywords_many_keywords() {
    let detail = "[keywords: a b c d e f g h i j] detail";
    let kw = extract_keywords(detail);
    assert_eq!(kw.len(), 10);
    assert_eq!(kw[0], "a");
    assert_eq!(kw[9], "j");
}

#[test]
fn test_extract_keywords_no_bracket_close() {
    let detail = "[keywords: read file";
    let kw = extract_keywords(detail);
    assert!(
        kw.is_empty(),
        "unclosed bracket should return empty vec, got {:?}",
        kw
    );
}

#[test]
fn test_extract_keywords_only_prefix_no_detail() {
    let detail = "[keywords: read file cat]";
    let kw = extract_keywords(detail);
    assert_eq!(kw, vec!["read", "file", "cat"]);
}

// =========================================================================
// build_descriptor keyword integration
// =========================================================================

struct KeywordDummyTool {
    name: String,
    detail_text: String,
}

impl Tool for KeywordDummyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        "test"
    }
    fn summary(&self) -> String {
        format!("summary for {}", self.name)
    }
    fn detail(&self) -> String {
        self.detail_text.clone()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn flags(&self) -> ToolFlags {
        ToolFlags::default()
    }
}

#[tokio::test]
async fn test_build_descriptor_extracts_keywords() {
    let reg = ToolRegistry::new();
    let tool = KeywordDummyTool {
        name: "TestTool".to_string(),
        detail_text: "[keywords: search find discover] some detail".to_string(),
    };
    reg.register(tool).await.unwrap();

    let desc = reg.get_tool_detail("TestTool").await.unwrap();
    assert_eq!(desc.keywords, vec!["search", "find", "discover"]);
}

#[tokio::test]
async fn test_build_descriptor_no_keywords_empty_vec() {
    let reg = ToolRegistry::new();
    let tool = KeywordDummyTool {
        name: "NoKeywords".to_string(),
        detail_text: "Just a regular detail".to_string(),
    };
    reg.register(tool).await.unwrap();

    let desc = reg.get_tool_detail("NoKeywords").await.unwrap();
    assert!(desc.keywords.is_empty());
}

#[tokio::test]
async fn test_get_tool_descriptors_populates_keywords() {
    let reg = ToolRegistry::new();
    reg.register(KeywordDummyTool {
        name: "A".to_string(),
        detail_text: "[keywords: alpha beta] detail a".to_string(),
    })
    .await
    .unwrap();
    reg.register(KeywordDummyTool {
        name: "B".to_string(),
        detail_text: "no keywords here".to_string(),
    })
    .await
    .unwrap();

    let descs = reg.get_tool_descriptors(None, None, None).await;
    assert_eq!(descs.len(), 2);
    let a = descs.iter().find(|d| d.name == "A").unwrap();
    assert_eq!(a.keywords, vec!["alpha", "beta"]);
    let b = descs.iter().find(|d| d.name == "B").unwrap();
    assert!(b.keywords.is_empty());
}
