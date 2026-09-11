//! Danger mark regression tests (Step 1.3).
//!
//! Verifies that `(read-only)` and `(destructive)` marks always appear
//! in `build_tools_section` output regardless of whether a tool is
/// eager or deferred. The design doc mandates these marks are always
/// shown — Step 1.2 changes (expensive mark migration) must not break
/// this existing behavior.
use super::*;

/// Helper: build a `ToolInfo` with configurable danger marks.
fn danger_tool_info(
    name: &str,
    group: &str,
    is_deferred: bool,
    is_read_only: bool,
    is_destructive: bool,
) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        group: group.to_string(),
        detail: format!("detail for {}", name),
        input_schema: serde_json::json!({}),
        is_deferred,
        is_read_only,
        is_destructive,
        is_expensive: false,
    }
}

// ── Eager tools: danger marks present ────────────────────────────────────────

/// Eager read-only tool → `(read-only)` appears in output.
#[test]
fn test_danger_eager_read_only() {
    let tool = danger_tool_info("ReadOnlyEager", "g", false, true, false);
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(read-only)"),
        "eager read-only should show (read-only), got: {output}"
    );
    assert!(
        output.contains("**ReadOnlyEager**"),
        "eager tool should be bold, got: {output}"
    );
}

/// Eager destructive tool → `(destructive)` appears in output.
#[test]
fn test_danger_eager_destructive() {
    let tool = danger_tool_info("DeleterEager", "g", false, false, true);
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(destructive)"),
        "eager destructive should show (destructive), got: {output}"
    );
}

// ── Deferred tools: danger marks present ─────────────────────────────────────

/// Deferred read-only tool → `(read-only)` appears (name only, no bold).
#[test]
fn test_danger_deferred_read_only() {
    let tool = danger_tool_info("ReadOnlyDeferred", "g", true, true, false);
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(read-only)"),
        "deferred read-only should show (read-only), got: {output}"
    );
    assert!(
        !output.contains("**ReadOnlyDeferred**"),
        "deferred tool should NOT be bold, got: {output}"
    );
    assert!(
        output.contains("  - ReadOnlyDeferred (read-only)"),
        "deferred format: name + (read-only), got: {output}"
    );
}

/// Deferred destructive tool → `(destructive)` appears.
#[test]
fn test_danger_deferred_destructive() {
    let tool = danger_tool_info("DeleterDeferred", "g", true, false, true);
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(destructive)"),
        "deferred destructive should show (destructive), got: {output}"
    );
    assert!(
        output.contains("  - DeleterDeferred (destructive)"),
        "deferred format: name + (destructive), got: {output}"
    );
}

// ── Deferred + expensive: danger marks still present, no expensive ──────────

/// Deferred read-only expensive tool → `(read-only)` present, no `(expensive)`.
#[test]
fn test_danger_deferred_read_only_expensive() {
    let tool = danger_tool_info("ReadOnlyExpensiveDeferred", "g", true, true, false);
    // Override expensive flag
    let mut tool = tool;
    tool.is_expensive = true;
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(read-only)"),
        "deferred read-only expensive should show (read-only), got: {output}"
    );
    assert!(
        !output.contains("(expensive)"),
        "deferred tool should NOT show (expensive) in first-level, got: {output}"
    );
}

/// Deferred destructive expensive tool → `(destructive)` present, no `(expensive)`.
#[test]
fn test_danger_deferred_destructive_expensive() {
    let mut tool = danger_tool_info("DestructiveExpensiveDeferred", "g", true, false, true);
    tool.is_expensive = true;
    let (lines, _) = ToolRegistryImpl::format_tool_line(&tool);
    let output = lines.join("\n");
    assert!(
        output.contains("(destructive)"),
        "deferred destructive expensive should show (destructive), got: {output}"
    );
    assert!(
        !output.contains("(expensive)"),
        "deferred tool should NOT show (expensive) in first-level, got: {output}"
    );
}

// ── Integration: build_tools_section with danger marks ───────────────────────

/// Full pipeline: register eager read-only and deferred destructive tools,
/// verify danger marks appear in `build_tools_section` output.
#[tokio::test]
async fn test_danger_marks_full_pipeline() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "EagerViewer".to_string(),
        group: "view".to_string(),
        summary_text: "view".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "DeferredEraser".to_string(),
        group: "erase".to_string(),
        summary_text: "erase".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: true,
        is_expensive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["EagerViewer", "DeferredEraser"]);
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        section.contains("**EagerViewer** (read-only)"),
        "eager read-only mark in full pipeline, got: {section}"
    );
    assert!(
        section.contains("  - DeferredEraser (destructive)"),
        "deferred destructive mark in full pipeline, got: {section}"
    );
}

/// Verify danger marks are present even when mixed with expensive tools.
#[tokio::test]
async fn test_danger_marks_with_expensive() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "ReadOnlyExpensive".to_string(),
        group: "g1".to_string(),
        summary_text: "roe".to_string(),
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
        summary_text: "de".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: true,
        is_expensive: true,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["ReadOnlyExpensive", "DestructiveExpensive"]);
    let section = reg.build_tools_section(&ctx).await;

    // Eager: (read-only) in danger mark, (expensive) in detail
    assert!(
        section.contains("**ReadOnlyExpensive** (read-only)"),
        "eager read-only + expensive: (read-only) present, got: {section}"
    );
    assert!(
        section.contains("(expensive)"),
        "eager expensive: (expensive) in detail, got: {section}"
    );
    // Deferred: (destructive) in danger mark, NO (expensive)
    assert!(
        section.contains("  - DestructiveExpensive (destructive)"),
        "deferred destructive + expensive: (destructive) present, got: {section}"
    );
    assert!(
        !section.contains("DestructiveExpensive (expensive)"),
        "deferred: should NOT have (expensive) in first-level, got: {section}"
    );
}
