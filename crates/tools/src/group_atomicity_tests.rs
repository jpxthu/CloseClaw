//! Group atomicity tests (Step 1.2).
//!
//! Verifies that `format_group_line` guarantees atomicity — a group either
//! outputs completely or is discarded entirely.

use super::*;

/// Helper to build a `ToolInfo` for group atomicity tests.
fn atomic_tool_info(name: &str, group: &str, is_deferred: bool) -> ToolInfo {
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

/// Case 1: Group completely within limit → complete output.
/// The header and all tool lines must appear in the output.
#[test]
fn test_group_atomicity_within_limit() {
    let tools = vec![
        atomic_tool_info("Alpha", "alpha_group", false),
        atomic_tool_info("Beta", "alpha_group", false),
    ];
    let (output, new_len) = ToolRegistryImpl::format_group_line("alpha_group", &tools, 0, 10000);
    assert!(
        !output.is_empty(),
        "group within limit should produce output"
    );
    assert!(
        output.contains("**alpha_group**"),
        "header present: {output}"
    );
    assert!(output.contains("**Alpha**"), "Alpha tool present: {output}");
    assert!(output.contains("**Beta**"), "Beta tool present: {output}");
    assert!(new_len > 0, "new_len should advance");
}

/// Case 2: Group exceeds limit → completely not output.
/// No header, no partial tools — the entire group is discarded.
#[test]
fn test_group_atomicity_exceeds_limit() {
    let tools = vec![
        atomic_tool_info("A", "overflow", false),
        atomic_tool_info("B", "overflow", false),
        atomic_tool_info("C", "overflow", false),
    ];
    // Header "**overflow** — (always loaded)" = 30 chars + 1 newline = 31.
    // Each eager tool line = ~24 chars. With max_len=50, header + 1 tool would
    // fit but atomicity requires all tools → entire group discarded.
    let (output, new_len) = ToolRegistryImpl::format_group_line("overflow", &tools, 0, 50);
    assert!(
        output.is_empty(),
        "group exceeding limit must produce no output"
    );
    assert_eq!(new_len, 0, "new_len unchanged when group discarded");
    assert!(!output.contains("**overflow**"), "no header: {output}");
    assert!(!output.contains("**A**"), "no tool A: {output}");
    assert!(!output.contains("**B**"), "no tool B: {output}");
    assert!(!output.contains("**C**"), "no tool C: {output}");
}

/// Case 3: Multiple groups — second exceeds limit → first complete, second absent.
/// Verifies that `build_tools_section` outputs group1 fully and skips group2
/// entirely (no partial group2 header or tools).
#[tokio::test]
async fn test_group_atomicity_second_group_skipped() {
    let reg = ToolRegistry::new();
    // group1: fits comfortably
    reg.register(DummyTool {
        name: "ShortTool".to_string(),
        group: "group1".to_string(),
        summary_text: "short".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    // group2: many tools → exceeds TOOLS_SECTION_MAX_LEN (15000 chars).
    // Each eager tool line ≈ 42 chars. 500 tools ≈ 21000 chars > 15000.
    for i in 0..500 {
        reg.register(DummyTool {
            name: format!("LongTool{i}"),
            group: "group2".to_string(),
            summary_text: format!("long tool {i}"),
            is_deferred: false,
            is_read_only: false,
            is_destructive: false,
        })
        .await
        .unwrap();
    }
    // group3: fits, but only if group2 is skipped (not break-loop)
    reg.register(DummyTool {
        name: "AnotherTool".to_string(),
        group: "group3".to_string(),
        summary_text: "another".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["ShortTool", "AnotherTool"]);
    let section = reg.build_tools_section(&ctx).await;

    // group1 complete output
    assert!(
        section.contains("**group1**"),
        "group1 header should appear, got: {section}"
    );
    assert!(
        section.contains("**ShortTool**"),
        "ShortTool should appear, got: {section}"
    );
    // group2 completely absent (no partial header or tools)
    assert!(
        !section.contains("**group2**"),
        "group2 header must NOT appear, got: {section}"
    );
    // group3 should appear (loop continues past skipped group2)
    assert!(
        section.contains("**group3**"),
        "group3 header should appear after group2 is skipped, got: {section}"
    );
    assert!(
        section.contains("**AnotherTool**"),
        "AnotherTool should appear, got: {section}"
    );
}
