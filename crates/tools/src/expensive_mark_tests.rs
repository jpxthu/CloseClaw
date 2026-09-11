//! Tests for the `(expensive)` annotation in tool rendering (Step 1.2).
//!
//! After Step 1.2 migration:
//! - Eager tools: `(expensive)` is embedded in the detail text (rendering layer).
//! - Deferred tools: first-level index shows name + danger mark only (no expensive).
//! - ToolSearch exact mode: secondary detail includes `(expensive)` annotation.

use super::*;

/// Helper: create a ToolInfo directly for format_tool_line testing.
fn make_tool_info(
    name: &str,
    is_read_only: bool,
    is_destructive: bool,
    is_expensive: bool,
) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        group: "test_group".to_string(),
        detail: format!("detail for {}", name),
        input_schema: serde_json::json!({}),
        is_deferred: false,
        is_read_only,
        is_destructive,
        is_expensive,
    }
}

/// Helper: create a deferred ToolInfo.
fn make_deferred_tool_info(
    name: &str,
    is_read_only: bool,
    is_destructive: bool,
    is_expensive: bool,
) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        group: "test_group".to_string(),
        detail: format!("detail for {}", name),
        input_schema: serde_json::json!({}),
        is_deferred: true,
        is_read_only,
        is_destructive,
        is_expensive,
    }
}

// --- format_tool_line: eager expensive ---

/// Eager expensive tool: (expensive) embedded in detail text, not as
/// separate line-level mark. Bold name preserved.
#[test]
fn test_format_tool_line_expensive_eager() {
    let tool = make_tool_info("Bash", false, false, true);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(expensive)"),
        "eager expensive tool should contain (expensive) in detail, got: {output}"
    );
    assert!(
        output.contains("**Bash**"),
        "eager tool should be bold, got: {output}"
    );
    // (expensive) should appear after the detail text, not as standalone mark
    assert!(
        output.contains("detail for Bash (expensive)"),
        "expensive should be appended to detail, got: {output}"
    );
}

/// Eager non-expensive tool: no (expensive) anywhere.
#[test]
fn test_format_tool_line_not_expensive_eager() {
    let tool = make_tool_info("Read", true, false, false);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        !output.contains("(expensive)"),
        "non-expensive tool should NOT contain (expensive), got: {output}"
    );
}

// --- format_tool_line: deferred expensive ---

/// Deferred expensive tool: first-level index line shows name + danger mark
/// only — no (expensive) annotation.
#[test]
fn test_format_tool_line_expensive_deferred() {
    let tool = make_deferred_tool_info("Search", false, false, true);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        !output.contains("(expensive)"),
        "deferred expensive tool should NOT have (expensive) in first-level index, got: {output}"
    );
    assert!(
        !output.contains("**Search**"),
        "deferred tool should NOT be bold, got: {output}"
    );
    assert!(
        output.contains("  - Search"),
        "deferred tool should show name, got: {output}"
    );
}

/// Deferred non-expensive tool: no (expensive), no bold.
#[test]
fn test_format_tool_line_not_expensive_deferred() {
    let tool = make_deferred_tool_info("Cleanup", false, false, false);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        !output.contains("(expensive)"),
        "non-expensive deferred tool should NOT contain (expensive), got: {output}"
    );
}

// --- format_tool_line: combination marks (danger + expensive) ---

/// Read-only + expensive: (read-only) in name, (expensive) in detail.
#[test]
fn test_format_tool_line_read_only_expensive() {
    let tool = make_tool_info("ReadOnlyExpensive", true, false, true);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(read-only)"),
        "read-only mark should be present, got: {output}"
    );
    assert!(
        output.contains("(expensive)"),
        "expensive should be in detail, got: {output}"
    );
    // Should NOT be on same line as (read-only) — expensive is in detail
    assert!(
        !output.contains("(read-only) (expensive)"),
        "read-only and expensive should NOT be adjacent marks, got: {output}"
    );
}

/// Destructive + expensive: (destructive) in name, (expensive) in detail.
#[test]
fn test_format_tool_line_destructive_expensive() {
    let tool = make_tool_info("DestructiveExpensive", false, true, true);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(destructive)"),
        "destructive mark should be present, got: {output}"
    );
    assert!(
        output.contains("(expensive)"),
        "expensive should be in detail, got: {output}"
    );
    assert!(
        !output.contains("(destructive) (expensive)"),
        "destructive and expensive should NOT be adjacent marks, got: {output}"
    );
}

/// Read-only, not expensive: (read-only) present, no (expensive).
#[test]
fn test_format_tool_line_read_only_not_expensive() {
    let tool = make_tool_info("ReadOnlyNormal", true, false, false);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(read-only)"),
        "read-only mark should be present, got: {output}"
    );
    assert!(
        !output.contains("(expensive)"),
        "non-expensive should NOT have (expensive), got: {output}"
    );
}

/// Destructive, not expensive: (destructive) present, no (expensive).
#[test]
fn test_format_tool_line_destructive_not_expensive() {
    let tool = make_tool_info("DestructiveNormal", false, true, false);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(destructive)"),
        "destructive mark should be present, got: {output}"
    );
    assert!(
        !output.contains("(expensive)"),
        "non-expensive should NOT have (expensive), got: {output}"
    );
}

// --- format_tool_line: no danger mark + expensive ---

/// No danger mark, expensive: only (expensive) in detail.
#[test]
fn test_format_tool_line_no_danger_expensive() {
    let tool = make_tool_info("NoDangerExpensive", false, false, true);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(expensive)"),
        "no-danger expensive should have (expensive), got: {output}"
    );
    assert!(
        !output.contains("(read-only)"),
        "should NOT have (read-only), got: {output}"
    );
    assert!(
        !output.contains("(destructive)"),
        "should NOT have (destructive), got: {output}"
    );
}

/// No marks at all: clean output.
#[test]
fn test_format_tool_line_no_marks() {
    let tool = make_tool_info("Plain", false, false, false);
    let (lines, _) = ToolRegistry::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        !output.contains("(read-only)"),
        "no-mark tool should NOT have (read-only), got: {output}"
    );
    assert!(
        !output.contains("(destructive)"),
        "no-mark tool should NOT have (destructive), got: {output}"
    );
    assert!(
        !output.contains("(expensive)"),
        "no-mark tool should NOT have (expensive), got: {output}"
    );
}

// --- build_index_raw: expensive mark ---

#[tokio::test]
async fn test_build_index_raw_expensive_tool() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Bash".to_string(),
        group: "exec".to_string(),
        summary_text: "Execute bash".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: true,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read file".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();

    let index = reg.build_index_raw().await;
    // Eager expensive: (expensive) in detail
    assert!(
        index.contains("(expensive)"),
        "expensive tool should produce (expensive) in detail, got: {index}"
    );
    assert!(
        index.contains("**Bash**"),
        "eager Bash should be bold in index, got: {index}"
    );
    assert!(
        index.contains("detail for Bash (expensive)"),
        "expensive should be appended to detail, got: {index}"
    );
    assert!(
        !index.contains("Read (expensive)"),
        "non-expensive Read should NOT have (expensive), got: {index}"
    );
}

#[tokio::test]
async fn test_build_index_raw_deferred_expensive_tool() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Search".to_string(),
        group: "search".to_string(),
        summary_text: "Search web".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
        is_expensive: true,
    })
    .await
    .unwrap();

    let index = reg.build_index_raw().await;
    // Deferred expensive: NO (expensive) in first-level index
    assert!(
        !index.contains("(expensive)"),
        "deferred expensive tool should NOT produce (expensive) in index, got: {index}"
    );
    assert!(
        !index.contains("**Search**"),
        "deferred Search should NOT be bold in index, got: {index}"
    );
    assert!(
        index.contains("  - Search"),
        "deferred tool should show name, got: {index}"
    );
}

#[tokio::test]
async fn test_build_index_raw_combination_marks() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "ReadOnlyExpensive".to_string(),
        group: "g1".to_string(),
        summary_text: "read only expensive".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
        is_expensive: true,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "DestructiveExpensive".to_string(),
        group: "g2".to_string(),
        summary_text: "destructive expensive".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: true,
        is_expensive: true,
    })
    .await
    .unwrap();

    let index = reg.build_index_raw().await;
    // (read-only) as danger mark, (expensive) in detail — not adjacent
    assert!(
        index.contains("(read-only)"),
        "read-only mark should be present, got: {index}"
    );
    assert!(
        index.contains("(expensive)"),
        "expensive should be in detail, got: {index}"
    );
    assert!(
        !index.contains("(read-only) (expensive)"),
        "marks should NOT be adjacent, got: {index}"
    );
    assert!(
        index.contains("(destructive)"),
        "destructive mark should be present, got: {index}"
    );
}

// --- build_tools_section: expensive mark in full pipeline ---

#[tokio::test]
async fn test_build_tools_section_expensive_mark() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Bash".to_string(),
        group: "exec".to_string(),
        summary_text: "Execute bash".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: true,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Search".to_string(),
        group: "search".to_string(),
        summary_text: "Search web".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
        is_expensive: true,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read file".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Bash", "Search", "Read"]);
    let section = reg.build_tools_section(&ctx).await;
    // Eager expensive tool: (expensive) in detail text
    assert!(
        section.contains("detail for Bash (expensive)"),
        "eager expensive should have (expensive) in detail, got: {section}"
    );
    assert!(
        section.contains("**Bash**"),
        "eager Bash should be bold, got: {section}"
    );
    // Deferred expensive tool: NO (expensive) in first-level index
    assert!(
        !section.contains("  - Search (expensive)"),
        "deferred expensive should NOT have (expensive) in index, got: {section}"
    );
    assert!(
        section.contains("  - Search"),
        "deferred Search should show name, got: {section}"
    );
    // Non-expensive tool: no (expensive)
    assert!(
        section.contains("**Read** (read-only):"),
        "read-only non-expensive should NOT have (expensive), got: {section}"
    );
    assert!(
        !section.contains("Read (expensive)"),
        "Read should NOT have (expensive), got: {section}"
    );
}
