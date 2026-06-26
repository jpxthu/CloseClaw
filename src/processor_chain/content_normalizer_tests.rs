use super::*;
use crate::im_adapter::normalized::{add_code_block_language_hint, normalize_urls};
use crate::processor_chain::context::{MessageContext, RawMessage};
use crate::processor_chain::processor::MessageProcessor;

// -------------------------------------------------------------------------
// strip_control_chars
// -------------------------------------------------------------------------

#[test]
fn test_strip_control_chars_ansi_escape() {
    let input = "\x1b[31mred text\x1b[0m";
    assert_eq!(strip_control_chars(input), "red text");
}

#[test]
fn test_strip_control_chars_ansi_complex() {
    let input = "\x1b[1;32mbold green\x1b[0m normal";
    assert_eq!(strip_control_chars(input), "bold green normal");
}

#[test]
fn test_strip_control_chars_null_byte() {
    let input = "hello\x00world";
    assert_eq!(strip_control_chars(input), "helloworld");
}

#[test]
fn test_strip_control_chars_preserves_newline() {
    let input = "line1\nline2";
    assert_eq!(strip_control_chars(input), "line1\nline2");
}

#[test]
fn test_strip_control_chars_preserves_tab() {
    let input = "col1\tcol2";
    assert_eq!(strip_control_chars(input), "col1\tcol2");
}

#[test]
fn test_strip_control_chars_preserves_cr() {
    let input = "line1\rline2";
    assert_eq!(strip_control_chars(input), "line1\rline2");
}

#[test]
fn test_strip_control_chars_control_chars_0x01_to_0x08() {
    let input = "a\x01b\x02c\x03d\x04e\x05f\x06g\x07h\x08i";
    assert_eq!(strip_control_chars(input), "abcdefghi");
}

#[test]
fn test_strip_control_chars_control_chars_0x0b_0x0c() {
    // 0x0B (vertical tab) and 0x0C (form feed) should be stripped
    let input = "a\x0Bb\x0Cc";
    assert_eq!(strip_control_chars(input), "abc");
}

#[test]
fn test_strip_control_chars_control_chars_0x0e_to_0x1f() {
    let input = "a\x0Eb\x0Fc\x10d\x11e\x12f\x13g";
    assert_eq!(strip_control_chars(input), "abcdefg");
}

#[test]
fn test_strip_control_chars_clean_text_unchanged() {
    let input = "Hello, world! 你好 🎉";
    assert_eq!(strip_control_chars(input), input);
}

#[test]
fn test_strip_control_chars_mixed() {
    let input = "\x1b[1mBold\x1b[0m and \x00normal";
    assert_eq!(strip_control_chars(input), "Bold and normal");
}

// -------------------------------------------------------------------------
// ContentNormalizer process()
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_process_plain_text() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "hello world".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_1".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap();
    assert!(result.is_some());
    let out = result.unwrap();
    assert_eq!(out.content, "hello world");
}

#[tokio::test]
async fn test_process_normalizes_empty_lines() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "hello\n\n\n\nworld  ".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_2".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "hello\n\nworld");
}

#[tokio::test]
async fn test_process_strips_ansi() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "\x1b[31mError:\x1b[0m something went wrong".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_3".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "Error: something went wrong");
}

#[tokio::test]
async fn test_process_strips_control_chars() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "hello\x00\x01\x02world".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_4".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "helloworld");
}

#[tokio::test]
async fn test_process_preserves_newlines_and_tabs() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "line1\nline2\ttab".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_5".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "line1\nline2\ttab");
}

// -----------------------------------------------------------------------
// ContentNormalizer does NOT normalize URLs or add code block language hints
// (these are handled by IM Adapter layer during parsing)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_process_does_not_normalize_urls() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "visit www.example.com today".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_url".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    // ContentNormalizer should NOT add https:// prefix
    assert_eq!(result.content, "visit www.example.com today");
}

#[tokio::test]
async fn test_process_does_not_normalize_bare_domain() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "go to google.com/path please".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_url2".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "go to google.com/path please");
}

#[tokio::test]
async fn test_process_does_not_add_code_block_language_hint() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "```\ncode here\n```".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_code".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    // ContentNormalizer should NOT add ```text language hint
    assert_eq!(result.content, "```\ncode here\n```");
    assert!(!result.content.contains("```text"));
}

#[tokio::test]
async fn test_process_combined_url_and_code_block_unchanged() {
    let processor = ContentNormalizer::new();
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "cli".to_string(),
        content: "see www.example.com and ```\nfn main() {}\n```".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_combo".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    // Neither URL nor code block should be modified
    assert_eq!(
        result.content,
        "see www.example.com and ```\nfn main() {}\n```"
    );
}

// -------------------------------------------------------------------------
// Markdown normalization functions
// -------------------------------------------------------------------------

#[test]
fn test_normalize_empty_lines_three_plus() {
    assert_eq!(
        normalize_empty_lines("hello\n\n\n\nworld"),
        "hello\n\nworld"
    );
}

#[test]
fn test_normalize_empty_lines_two() {
    assert_eq!(normalize_empty_lines("hello\n\nworld"), "hello\n\nworld");
}

#[test]
fn test_normalize_empty_lines_single() {
    let input = "hello\nworld";
    let out = normalize_empty_lines(input);
    assert_eq!(out, "hello\nworld");
}

#[test]
fn test_trim_trailing_whitespace() {
    assert_eq!(
        trim_trailing_whitespace("hello   \nworld  "),
        "hello\nworld"
    );
}

#[test]
fn test_trim_trailing_whitespace_without_space() {
    let input = "hello\nworld";
    let out = trim_trailing_whitespace(input);
    assert_eq!(out, "hello\nworld");
}

#[test]
fn test_normalize_urls_www() {
    assert_eq!(
        normalize_urls("then www.example.com also"),
        "then https://www.example.com also"
    );
}

#[test]
fn test_normalize_urls_bare_domain() {
    assert_eq!(
        normalize_urls("visit google.com/path please"),
        "visit https://google.com/path please"
    );
}

#[test]
fn test_normalize_urls_http_unchanged() {
    assert_eq!(
        normalize_urls("see http://example.com ok"),
        "see http://example.com ok"
    );
}

#[test]
fn test_normalize_urls_in_markdown_link_unchanged() {
    let input = "see [example](www.example.com) link";
    let out = normalize_urls(input);
    assert_eq!(out, "see [example](www.example.com) link", "got: {out}");
}

#[test]
fn test_add_language_hint_unlabeled() {
    let out = add_code_block_language_hint("```\ncode here\n```");
    assert!(out.contains("```text"), "got: {out}");
}

#[test]
fn test_add_language_hint_labeled_unchanged() {
    let out = add_code_block_language_hint("```rust\nfn main() {}\n```");
    assert!(out.contains("```rust"));
}

#[test]
fn test_add_code_block_normal_text_unchanged() {
    let input = "just some plain text";
    let out = add_code_block_language_hint(input);
    assert_eq!(out, "just some plain text");
}

// -------------------------------------------------------------------------
// strip_platform_residue
// -------------------------------------------------------------------------

#[test]
fn test_strip_platform_residue_single_at_tag() {
    let input = r#"Hello <at user_id="u123">Alice</at>, how are you?"#;
    assert_eq!(strip_platform_residue(input), "Hello @Alice, how are you?");
}

#[test]
fn test_strip_platform_residue_multiple_at_tags() {
    let input = r#"<at user_id="u1">Alice</at> and <at user_id="u2">Bob</at> are here"#;
    assert_eq!(strip_platform_residue(input), "@Alice and @Bob are here");
}

#[test]
fn test_strip_platform_residue_adjacent_at_tags() {
    let input = r#"<at user_id="u1">A</at><at user_id="u2">B</at>"#;
    assert_eq!(strip_platform_residue(input), "@A@B");
}

#[test]
fn test_strip_platform_residue_plain_text_unchanged() {
    let input = "Hello world, no tags here.";
    assert_eq!(strip_platform_residue(input), input);
}

#[test]
fn test_strip_platform_residue_empty_string() {
    assert_eq!(strip_platform_residue(""), "");
}

#[test]
fn test_strip_platform_residue_incomplete_tag_no_close() {
    let input = r#"Hello <at user_id="u1">Alice"#;
    assert_eq!(strip_platform_residue(input), input);
}

#[test]
fn test_strip_platform_residue_incomplete_tag_no_content() {
    let input = r#"Hello <at user_id="u1"></at>world"#;
    assert_eq!(strip_platform_residue(input), "Hello @world");
}

#[test]
fn test_strip_platform_residue_special_chars_in_name() {
    let input = r#"<at user_id="u1">O'Brien & Co.</at> said hi"#;
    assert_eq!(strip_platform_residue(input), "@O'Brien & Co. said hi");
}

#[test]
fn test_strip_platform_residue_unicode_name() {
    let input = r#"<at user_id="u1">张三</at> posted"#;
    assert_eq!(strip_platform_residue(input), "@张三 posted");
}

#[test]
fn test_strip_platform_residue_emoji_name() {
    let input = r#"<at user_id="u1">🎉 Party</at> time"#;
    assert_eq!(strip_platform_residue(input), "@🎉 Party time");
}

#[test]
fn test_strip_platform_residue_at_in_other_context() {
    // An @ that is NOT inside an <at> tag should be left alone
    let input = "email user@example.com or ping <at user_id=\"u1\">Alice</at>";
    assert_eq!(
        strip_platform_residue(input),
        "email user@example.com or ping @Alice"
    );
}

#[test]
fn test_strip_platform_residue_only_at_tag() {
    let input = r#"<at user_id="u1">Solo</at>"#;
    assert_eq!(strip_platform_residue(input), "@Solo");
}

#[test]
fn test_strip_platform_residue_nested_xml_like_surrounding() {
    // <at> tag embedded in other XML-like text — the regex should still match
    let input = r#"<root><at user_id="u1">Alice</at></root>"#;
    assert_eq!(strip_platform_residue(input), "<root>@Alice</root>");
}
