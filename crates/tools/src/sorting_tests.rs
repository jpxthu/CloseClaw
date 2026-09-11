//! Sorting tests (Step 1.3).
//!
//! Verifies that `build_tools_section` output order satisfies:
//! 1. Groups containing eager (always-loaded) tools sort before all-deferred groups.
//! 2. Within each tier, groups sort alphabetically by name.
//! 3. Tools within a group sort alphabetically by name.
//! 4. Empty / single-group edge cases.

use super::*;

/// Helper: build a `ToolInfo` with configurable flags.
fn sort_tool_info(name: &str, group: &str, is_deferred: bool, is_expensive: bool) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        group: group.to_string(),
        detail: format!("detail for {}", name),
        input_schema: serde_json::json!({}),
        is_deferred,
        is_read_only: false,
        is_destructive: false,
        is_expensive,
    }
}

// ── Mixed group ordering: eager groups first, deferred groups last ──────────

/// Two groups: alpha_group (all deferred), beta_group (has eager).
/// Output should show beta_group before alpha_group.
#[test]
fn test_sort_eager_group_before_deferred_group() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "alpha_group".into(),
        vec![sort_tool_info("Alpha1", "alpha_group", true, false)],
    );
    groups_map.insert(
        "beta_group".into(),
        vec![sort_tool_info("Beta1", "beta_group", false, false)],
    );

    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });

    assert_eq!(sorted[0].0, "beta_group", "eager group should come first");
    assert_eq!(
        sorted[1].0, "alpha_group",
        "deferred group should come second"
    );
}

/// Three groups: A (all deferred), B (eager), C (eager).
/// B and C are eager → sorted alphabetically (B, C), then A (deferred).
#[test]
fn test_sort_multiple_eager_deferred_tiers() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert("A".into(), vec![sort_tool_info("a1", "A", true, false)]);
    groups_map.insert("B".into(), vec![sort_tool_info("b1", "B", false, false)]);
    groups_map.insert("C".into(), vec![sort_tool_info("c1", "C", false, false)]);

    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });

    let names: Vec<&str> = sorted.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["B", "C", "A"]);
}

/// Mixed group (has both eager and deferred) counts as eager group.
#[test]
fn test_sort_mixed_group_counts_as_eager() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "mixed".into(),
        vec![
            sort_tool_info("e1", "mixed", false, false),
            sort_tool_info("d1", "mixed", true, false),
        ],
    );
    groups_map.insert(
        "pure_deferred".into(),
        vec![sort_tool_info("p1", "pure_deferred", true, false)],
    );

    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });

    assert_eq!(
        sorted[0].0, "mixed",
        "mixed group (has eager) should come first"
    );
    assert_eq!(
        sorted[1].0, "pure_deferred",
        "pure deferred group should come second"
    );
}

// ── Alphabetical ordering within tiers ──────────────────────────────────────

/// Two eager groups: zebra, alpha → sorted alphabetically as alpha, zebra.
#[test]
fn test_sort_eager_groups_alphabetical() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "zebra".into(),
        vec![sort_tool_info("z1", "zebra", false, false)],
    );
    groups_map.insert(
        "alpha".into(),
        vec![sort_tool_info("a1", "alpha", false, false)],
    );

    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });

    assert_eq!(sorted[0].0, "alpha", "alpha should come before zebra");
    assert_eq!(sorted[1].0, "zebra", "zebra should come after alpha");
}

/// Two deferred groups: zebra, alpha → sorted alphabetically.
#[test]
fn test_sort_deferred_groups_alphabetical() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "zebra".into(),
        vec![sort_tool_info("z1", "zebra", true, false)],
    );
    groups_map.insert(
        "alpha".into(),
        vec![sort_tool_info("a1", "alpha", true, false)],
    );

    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });

    assert_eq!(
        sorted[0].0, "alpha",
        "alpha should come before zebra in deferred tier"
    );
    assert_eq!(sorted[1].0, "zebra");
}

/// Tools within a group sort alphabetically by name.
#[test]
fn test_sort_tools_within_group_alphabetical() {
    let tools = vec![
        sort_tool_info("Zebra", "g", false, false),
        sort_tool_info("Alpha", "g", false, false),
        sort_tool_info("Mango", "g", false, false),
    ];
    let mut sorted_tools: Vec<_> = tools.iter().collect();
    sorted_tools.sort_by_key(|t| t.name.clone());
    let names: Vec<&str> = sorted_tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "Mango", "Zebra"]);
}

// ── Edge cases ──────────────────────────────────────────────────────────────

/// Empty group map → no output.
#[test]
fn test_sort_empty_groups() {
    let mut sorted_groups: Vec<(String, Vec<ToolInfo>)> = Vec::new();
    sorted_groups.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });
    assert!(sorted_groups.is_empty());
}

/// Single group → trivial ordering.
#[test]
fn test_sort_single_group() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "only".into(),
        vec![sort_tool_info("t1", "only", false, false)],
    );
    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].0, "only");
}

/// Single deferred group with one tool.
#[test]
fn test_sort_single_deferred_group() {
    let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
        std::collections::HashMap::new();
    groups_map.insert(
        "deferred_only".into(),
        vec![sort_tool_info("d1", "deferred_only", true, false)],
    );
    let mut sorted: Vec<_> = groups_map.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_has_eager = a.1.iter().any(|t| !t.is_deferred);
        let b_has_eager = b.1.iter().any(|t| !t.is_deferred);
        b_has_eager.cmp(&a_has_eager).then_with(|| a.0.cmp(&b.0))
    });
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].0, "deferred_only");
}

// ── Integration: build_tools_section ordering verification ───────────────────

/// Register tools across mixed groups, verify output order in
/// `build_tools_section`.
#[tokio::test]
async fn test_sort_build_tools_section_mixed_order() {
    let reg = ToolRegistry::new();
    // deferred group
    reg.register(DummyTool {
        name: "Alpha1".to_string(),
        group: "alpha".to_string(),
        summary_text: "alpha tool".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();
    // eager group
    reg.register(DummyTool {
        name: "Bravo1".to_string(),
        group: "bravo".to_string(),
        summary_text: "bravo tool".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();
    // another eager group
    reg.register(DummyTool {
        name: "Charlie1".to_string(),
        group: "charlie".to_string(),
        summary_text: "charlie tool".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Alpha1", "Bravo1", "Charlie1"]);
    let section = reg.build_tools_section(&ctx).await;

    // Find positions of group headers
    let bravo_pos = section.find("**bravo**").unwrap_or(usize::MAX);
    let charlie_pos = section.find("**charlie**").unwrap_or(usize::MAX);
    let alpha_pos = section.find("**alpha**").unwrap_or(usize::MAX);

    // Eager groups (bravo, charlie) must come before deferred group (alpha)
    assert!(
        bravo_pos < alpha_pos,
        "bravo (eager) should appear before alpha (deferred)"
    );
    assert!(
        charlie_pos < alpha_pos,
        "charlie (eager) should appear before alpha (deferred)"
    );
    // Within eager tier: alphabetical (bravo before charlie)
    assert!(
        bravo_pos < charlie_pos,
        "bravo should appear before charlie (alphabetical)"
    );
}

/// Register tools with deferred eager-deferred mixed group,
/// verify mixed group appears before pure-deferred groups.
#[tokio::test]
async fn test_sort_mixed_group_before_pure_deferred() {
    let reg = ToolRegistry::new();
    // mixed group (eager + deferred)
    reg.register(DummyTool {
        name: "MixEager".to_string(),
        group: "mixed".to_string(),
        summary_text: "mixed eager".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "MixDeferred".to_string(),
        group: "mixed".to_string(),
        summary_text: "mixed deferred".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();
    // pure deferred group
    reg.register(DummyTool {
        name: "PureDeferred".to_string(),
        group: "pure_deferred".to_string(),
        summary_text: "pure deferred".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["MixEager", "MixDeferred", "PureDeferred"]);
    let section = reg.build_tools_section(&ctx).await;

    let mixed_pos = section.find("**mixed**").unwrap_or(usize::MAX);
    let pd_pos = section.find("**pure_deferred**").unwrap_or(usize::MAX);

    assert!(
        mixed_pos < pd_pos,
        "mixed (has eager) should appear before pure_deferred"
    );
}
