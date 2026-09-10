//! ToolsSection truncation tests (Step 1.3).

use super::*;

/// Helper to create a ToolInfo for truncation tests.
fn trunc_tool_info(name: &str, group: &str, is_deferred: bool) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        group: group.to_string(),
        detail: format!("detail for {}", name),
        input_schema: serde_json::json!({}),
        is_deferred,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    }
}

/// Normal path: tools within limit → all tools shown.
#[test]
fn test_trunc_all_tools_within_limit() {
    let tools = vec![
        trunc_tool_info("A", "g", false),
        trunc_tool_info("B", "g", false),
    ];
    let (output, _) = ToolRegistryImpl::format_group_line("g", &tools, 0, 10000);
    assert!(output.contains("**A**"), "A should be present");
    assert!(output.contains("**B**"), "B should be present");
}

/// Boundary: tools at exact limit → all tools shown.
#[test]
fn test_trunc_exact_limit() {
    let tools = vec![
        trunc_tool_info("A", "g", false),
        trunc_tool_info("B", "g", false),
    ];
    let (_, full_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, usize::MAX);
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, full_len);
    assert!(output.contains("**A**"), "A at exact limit");
    assert!(output.contains("**B**"), "B at exact limit");
    assert_eq!(new_len, full_len);
}

/// Atomicity: group over limit → entire group discarded (no partial output).
#[test]
fn test_trunc_partial_group() {
    let tools = vec![
        trunc_tool_info("A", "g", false),
        trunc_tool_info("B", "g", false),
        trunc_tool_info("C", "g", false),
    ];
    // Header (**g** — (always loaded)) = 26 chars + newline = 27.
    // Each eager tool line = ~24 chars. With max_len=51 only header + 1 tool fit,
    // but atomicity requires the entire group to fit → nothing returned.
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, 51);
    assert!(output.is_empty(), "group discarded when not all tools fit");
    assert_eq!(new_len, 0, "new_len unchanged when group discarded");
    assert!(!output.contains("**g**"), "no header when group discarded");
    assert!(!output.contains("**A**"), "no tools when group discarded");
}

/// Extreme: max_len too small for any tool → nothing returned.
#[test]
fn test_trunc_extreme_no_tools_fit() {
    let tools = vec![
        trunc_tool_info("A", "g", false),
        trunc_tool_info("B", "g", false),
    ];
    // max_len=0 → already at limit, return empty
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, 0);
    assert!(output.is_empty(), "empty when max_len=0");
    assert_eq!(new_len, 0, "new_len=0 when max_len=0");
    assert!(!output.contains("**g**"), "no header with max_len=0");
    assert!(!output.contains("**A**"), "no tools with max_len=0");
    assert!(!output.contains("**B**"), "no tools with max_len=0");
}

/// Header overflow: header alone exceeds max_len → return empty.
#[test]
fn test_trunc_header_overflow_returns_empty() {
    let tools = vec![trunc_tool_info("A", "g", false)];
    // Header "**g** — (always loaded)" = 23 chars + 1 newline = 24.
    // Set max_len = 23 so header alone (with \n) doesn't fit.
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, 23);
    assert!(output.is_empty(), "header overflow returns empty");
    assert_eq!(new_len, 0, "new_len unchanged");
}

/// Header fits exactly, no room for tools → entire group discarded.
#[test]
fn test_trunc_header_fits_no_tools_fit() {
    let tools = vec![trunc_tool_info("A", "g", false)];
    // Header "**g** — (always loaded)\n" = 24 chars.
    // max_len = 24 → header fits but tool A doesn't → entire group discarded.
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, 24);
    assert!(output.is_empty(), "group discarded when tools don't fit");
    assert_eq!(new_len, 0, "new_len unchanged when group discarded");
    assert!(!output.contains("**g**"), "no header when group discarded");
    assert!(!output.contains("**A**"), "no tool when group discarded");
}

/// Simulates multi-group: front group consumes space,
/// back group discarded entirely (atomicity).
#[test]
fn test_trunc_simulated_multi_group() {
    let back = vec![
        trunc_tool_info("BA", "back", false),
        trunc_tool_info("BB", "back", false),
        trunc_tool_info("BC", "back", false),
    ];
    // Back header **back** — (always loaded) = 30 chars + newline = 31.
    // Each eager tool line = 26 chars + newline = 27 chars.
    // With total_len=50: header ends at 81, BA at 108, BB at 135.
    // Set max_len=110 → header + BA would fit but BB doesn't → entire group discarded.
    let (output, new_len) = ToolRegistryImpl::format_group_line("back", &back, 50, 110);
    assert!(
        output.is_empty(),
        "back group discarded when not all tools fit"
    );
    assert_eq!(new_len, 50, "new_len unchanged when back group discarded");
    assert!(
        !output.contains("**back**"),
        "no header when group discarded"
    );
    assert!(!output.contains("**BA**"), "no tools when group discarded");
}

// ---- split_long_line UTF-8 off-by-one tests ----

/// Multi-byte UTF-8: no-space split should not produce a line wider than
/// `width` characters. Uses `char_indices().take(width).last()` to avoid
/// the off-by-one of `.nth(width)`.
#[test]
fn test_split_long_line_multibyte_no_space() {
    // 5 multi-byte chars (3 bytes each = 15 bytes). Width=3 → should keep 3 chars.
    let line = "αβγδε";
    let chunks = ToolRegistryImpl::split_long_line(line, 3);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].chars().count(), 3, "first chunk has 3 chars");
    assert_eq!(chunks[0], "αβγ");
    assert_eq!(chunks[1].chars().count(), 2, "second chunk has 2 chars");
    assert_eq!(chunks[1], "δε");
}

/// ASCII no-space: split at width boundary.
#[test]
fn test_split_long_line_ascii_no_space() {
    let line = "abcdefghij";
    let chunks = ToolRegistryImpl::split_long_line(line, 4);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0], "abcd");
    assert_eq!(chunks[1], "efgh");
    assert_eq!(chunks[2], "ij");
}

/// Short line stays intact.
#[test]
fn test_split_long_line_short() {
    let line = "hello";
    let chunks = ToolRegistryImpl::split_long_line(line, 10);
    assert_eq!(chunks, vec!["hello"]);
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.2 — truncation hint tests
// ═══════════════════════════════════════════════════════════════════════════

/// Long-detail DummyTool that produces a known-length detail string.
struct LongDetailTool {
    name: String,
    group: String,
    detail_len: usize,
    is_deferred: bool,
}

impl Tool for LongDetailTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        &self.group
    }
    fn summary(&self) -> String {
        self.name.clone()
    }
    fn detail(&self) -> String {
        // Pad to exact length with 'x' chars.
        let base = format!("{} detail ", self.name);
        if base.len() >= self.detail_len {
            base
        } else {
            let padding = self.detail_len - base.len();
            format!("{}{}", base, "x".repeat(padding))
        }
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn flags(&self) -> ToolFlags {
        let mut f = ToolFlags::default();
        f.is_deferred_by_default = self.is_deferred;
        f
    }
}

/// Register enough tools so some groups are truncated, verify output
/// contains the truncation hint message.
#[tokio::test]
async fn test_build_tools_section_truncation_hint() {
    let reg = ToolRegistry::new();
    // First group: small, fits easily.
    reg.register(DummyTool {
        name: "Small1".to_string(),
        group: "small".to_string(),
        summary_text: "s1".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Second group: many tools with long details, will exceed limit.
    // 50 tools * ~340 chars each ≈ 17000 > TOOLS_SECTION_MAX_LEN (15000).
    for i in 0..50 {
        reg.register(LongDetailTool {
            name: format!("Bulk{i}"),
            group: "bulk".to_string(),
            detail_len: 300,
            is_deferred: false,
        })
        .await
        .unwrap();
    }
    let names: Vec<String> = (0..50)
        .map(|i| format!("Bulk{i}"))
        .chain(std::iter::once("Small1".into()))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let ctx = make_prompt_ctx(&refs);
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("Some tools were omitted"),
        "expected truncation hint, got: {section}"
    );
    assert!(
        section.contains("ToolSearch"),
        "hint should mention ToolSearch, got: {section}"
    );
    // The first group (small) should still be present.
    assert!(
        section.contains("**small**"),
        "small group should be present, got: {section}"
    );
}

/// All groups within limit → output must NOT contain the truncation hint.
#[tokio::test]
async fn test_build_tools_section_no_hint_when_no_truncation() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read files".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write files".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "ToolSearch".to_string(),
        group: "meta".to_string(),
        summary_text: "Search tools".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    let ctx = make_prompt_ctx(&["Read", "Write", "ToolSearch"]);
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        !section.contains("Some tools were omitted"),
        "no truncation expected, got: {section}"
    );
    assert!(
        section.contains("**file_ops**"),
        "file_ops group should be present, got: {section}"
    );
}

/// Verify the hint is counted toward TOOLS_SECTION_MAX_LEN and the total
/// output never exceeds the limit.
#[tokio::test]
async fn test_build_tools_section_hint_respects_max_len() {
    let reg = ToolRegistry::new();
    // One small group that fits, one bulk group that gets truncated.
    reg.register(DummyTool {
        name: "Small1".to_string(),
        group: "small".to_string(),
        summary_text: "s1".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    for i in 0..50 {
        reg.register(LongDetailTool {
            name: format!("Bulk{i}"),
            group: "bulk".to_string(),
            detail_len: 300,
            is_deferred: false,
        })
        .await
        .unwrap();
    }
    let names: Vec<String> = (0..50)
        .map(|i| format!("Bulk{i}"))
        .chain(std::iter::once("Small1".into()))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let ctx = make_prompt_ctx(&refs);
    let section = reg.build_tools_section(&ctx).await;
    let hint = concat!(
        "\nSome tools were omitted due to length limits. ",
        "Use ToolSearch to discover them by name or keyword.\n",
    );
    // Hint must be present.
    assert!(
        section.contains(hint),
        "hint must be present, got: {section}"
    );
    // Total section length must not exceed TOOLS_SECTION_MAX_LEN.
    assert!(
        section.len() <= TOOLS_SECTION_MAX_LEN,
        "section len {} must be <= {TOOLS_SECTION_MAX_LEN}",
        section.len()
    );
    // The hint itself must not be truncated — it must appear in full.
    assert!(
        section.ends_with("\n"),
        "section should end with newline, got: {section}"
    );
}
