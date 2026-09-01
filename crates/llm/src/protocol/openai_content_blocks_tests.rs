//! Step 1.6: content_blocks serialization tests for OpenAI protocol.
//! Extracted from openai_tests.rs to stay under 1000-line limit.

use crate::types::{ContentBlock as CBlock, InternalMessage};

/// Text + Image blocks → OpenAI content array with image_url.
#[test]
fn test_build_message_content_blocks_text_and_image() {
    let msg = InternalMessage {
        role: "user".to_string(),
        content: String::new(),
        content_blocks: Some(vec![
            CBlock::Text("describe this".to_string()),
            CBlock::Image {
                name: "photo.jpg".to_string(),
                url: "data:image/jpeg;base64,/9j/4AAQ".to_string(),
            },
        ]),
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "user");
    let content = value["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    // Text block
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "describe this");
    // Image block
    assert_eq!(content[1]["type"], "image_url");
    let image_url = &content[1]["image_url"];
    assert_eq!(image_url["url"], "data:image/jpeg;base64,/9j/4AAQ");
}

/// Empty content_blocks → falls back to plain content string.
#[test]
fn test_build_message_empty_content_blocks_falls_back() {
    let msg = InternalMessage {
        role: "user".to_string(),
        content: "hello".to_string(),
        content_blocks: Some(vec![]),
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "user");
    assert_eq!(value["content"], "hello");
    assert!(value["content"].is_string());
}

/// No content_blocks → plain content string (backward compat).
#[test]
fn test_build_message_no_content_blocks_plain_string() {
    let msg = InternalMessage {
        role: "assistant".to_string(),
        content: "I can help with that.".to_string(),
        content_blocks: None,
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "assistant");
    assert_eq!(value["content"], "I can help with that.");
    assert!(value["content"].is_string());
}

/// Multiple images → all serialized as image_url blocks.
#[test]
fn test_build_message_multiple_image_blocks() {
    let msg = InternalMessage {
        role: "user".to_string(),
        content: String::new(),
        content_blocks: Some(vec![
            CBlock::Image {
                name: "a.png".to_string(),
                url: "data:image/png;base64,AAAA".to_string(),
            },
            CBlock::Image {
                name: "b.png".to_string(),
                url: "data:image/png;base64,BBBB".to_string(),
            },
        ]),
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    let content = value["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "image_url");
    assert_eq!(content[0]["image_url"]["url"], "data:image/png;base64,AAAA");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,BBBB");
}

/// Tool result message ignores content_blocks (uses tool_call_id path).
#[test]
fn test_build_message_tool_result_ignores_content_blocks() {
    let msg = InternalMessage {
        role: "tool".to_string(),
        content: "result".to_string(),
        content_blocks: Some(vec![CBlock::Image {
            name: "x".to_string(),
            url: "data:image/png;base64,Y".to_string(),
        }]),
        tool_call_id: Some("tc_1".to_string()),
    };
    let value = super::build_message(&msg);
    // Tool result path takes precedence
    assert_eq!(value["role"], "tool");
    assert_eq!(value["content"], "result");
    // Should be plain string, not content array
    assert!(value["content"].is_string());
}
