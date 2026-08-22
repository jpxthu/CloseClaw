//! Unit tests for Feishu adapter: post emoji tag handling.

use super::*;

// ===========================================================================
// emoji tag handler tests (Step 1.2)
// ===========================================================================

#[test]
fn test_expand_post_emoji_ok() {
    let content = serde_json::json!({
        "content": [[{"tag": "emoji", "emoji_type": "OK"}]]
    });
    assert_eq!(expand_post_content(&content), "[OK]");
}

#[test]
fn test_expand_post_emoji_zan() {
    let content = serde_json::json!({
        "content": [[{"tag": "emoji", "emoji_type": "赞"}]]
    });
    assert_eq!(expand_post_content(&content), "[赞]");
}

#[test]
fn test_expand_post_emoji_empty_type() {
    let content = serde_json::json!({
        "content": [[{"tag": "emoji", "emoji_type": ""}]]
    });
    assert_eq!(expand_post_content(&content), "[未知消息]");
}

#[test]
fn test_expand_post_emoji_missing_type() {
    let content = serde_json::json!({
        "content": [[{"tag": "emoji"}]]
    });
    assert_eq!(expand_post_content(&content), "[未知消息]");
}

#[test]
fn test_expand_post_emoji_mixed_with_other_elements() {
    let content = serde_json::json!({
        "title": "Mixed",
        "content": [
            [{"tag": "text", "text": "Hello "}, {"tag": "emoji", "emoji_type": "thumbsup"}],
            [{"tag": "text", "text": "and "}, {"tag": "emoji", "emoji_type": "赞"}, {"tag": "text", "text": " too"}],
            [{"tag": "at", "name": "Alice"}, {"tag": "emoji", "emoji_type": "heart"}]
        ]
    });
    assert_eq!(
        expand_post_content(&content),
        "Mixed\nHello [thumbsup]\nand [赞] too\n@Alice[heart]"
    );
}
