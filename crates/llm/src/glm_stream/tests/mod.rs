//! Unit tests for GLM streaming SSE line parsing.

use crate::glm_stream::parse_sse_line;

// ---------------------------------------------------------------------------
// parse_sse_line edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_parse_sse_line_empty_line() {
    assert_eq!(parse_sse_line(""), None);
    assert_eq!(parse_sse_line("   "), None);
    assert_eq!(parse_sse_line("\t"), None);
}

#[test]
fn test_parse_sse_line_non_data_line() {
    assert_eq!(parse_sse_line("event: message"), None);
    assert_eq!(parse_sse_line(":id: 123"), None);
    assert_eq!(parse_sse_line("comment: hello"), None);
}

#[test]
fn test_parse_sse_line_done() {
    assert_eq!(parse_sse_line("data: [DONE]"), Some("[DONE]"));
    assert_eq!(parse_sse_line("data:    [DONE]"), Some("[DONE]"));
}

#[test]
fn test_parse_sse_line_data_json() {
    let line = r#"data: {"id":"abc","choices":[]}"#;
    let parsed = parse_sse_line(line);
    assert!(parsed.is_some());
    let data = parsed.unwrap();
    assert!(data.starts_with('{'));
    assert!(data.contains("\"id\":\"abc\""));
}

#[test]
fn test_parse_sse_line_non_json_data_prefix() {
    // e.g. "data: hello world" — not JSON, should be skipped
    assert_eq!(parse_sse_line("data: hello world"), None);
    assert_eq!(parse_sse_line("data: 123"), None);
}

#[test]
fn test_parse_sse_line_whitespace_prefix() {
    let line = "  data: {\"id\":\"abc\"}";
    assert_eq!(parse_sse_line(line), Some("{\"id\":\"abc\"}"));
}

#[test]
fn test_parse_sse_line_delta_with_reasoning_content() {
    let line = r#"data: {"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"thinking..."}}]}"#;
    let data = parse_sse_line(line).expect("should parse");
    assert!(data.contains("reasoning_content"));
    assert!(data.contains("thinking..."));
}

#[test]
fn test_parse_sse_line_final_with_usage() {
    let line = r#"data: {"id":"abc","model":"glm-5.1","choices":[{"index":0,"finish_reason":"length","delta":{"role":"assistant","content":""}}],"usage":{"prompt_tokens":12,"completion_tokens":50,"total_tokens":62}}"#;
    let data = parse_sse_line(line).expect("should parse");
    assert!(data.contains("finish_reason"));
    assert!(data.contains("usage"));
}

#[test]
fn test_parse_sse_line_glm_error() {
    let line = r#"data: {"error":{"code":"1211","message":"模型不存在"}}"#;
    let data = parse_sse_line(line).expect("should parse");
    assert!(data.contains("1211"));
    assert!(data.contains("模型不存在"));
}
