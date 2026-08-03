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

/// Truncation: tools over limit → partial tools shown.
#[test]
fn test_trunc_partial_group() {
    let tools = vec![
        trunc_tool_info("A", "g", false),
        trunc_tool_info("B", "g", false),
        trunc_tool_info("C", "g", false),
    ];
    // Header (**g** — (always loaded)) = 26 chars + newline = 27.
    // Each eager tool line = ~24 chars. With max_len=51 only header + 1 tool fit.
    let (output, new_len) = ToolRegistryImpl::format_group_line("g", &tools, 0, 51);
    assert!(output.contains("**g**"), "header present");
    assert!(output.contains("**A**"), "first tool present");
    assert!(!output.contains("**B**"), "second tool truncated");
    assert!(!output.contains("**C**"), "third tool truncated");
    assert!(new_len <= 51, "new_len must not exceed max_len");
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

/// Simulates multi-group truncation: front group consumes space,
/// back group is truncated.
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
    // Set max_len=110 → header + BA fit, BB does not.
    let (output, new_len) = ToolRegistryImpl::format_group_line("back", &back, 50, 110);
    assert!(output.contains("**back**"), "back header present");
    assert!(output.contains("**BA**"), "first tool present");
    assert!(!output.contains("**BB**"), "second tool truncated");
    assert!(!output.contains("**BC**"), "third tool truncated");
    let tool_count = output.lines().filter(|l| l.starts_with("  - ")).count();
    assert_eq!(tool_count, 1, "exactly one tool in truncated group");
    assert!(new_len <= 110, "new_len must not exceed max_len");
}
