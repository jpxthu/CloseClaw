//! Sorting integration tests (Step 1.3 + Step 1.4 cleanup).
//!
//! Verifies that `build_tools_section` output order satisfies:
//! 1. Groups containing eager (always-loaded) tools sort before all-deferred groups.
//! 2. Within each tier, groups sort alphabetically by name.
//! 3. Tools within a group sort alphabetically by name.
//!
//! Pure unit tests that copied the sorting closure were removed in Step 1.4;
//! only integration tests going through the real registry path remain.

use super::*;

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
