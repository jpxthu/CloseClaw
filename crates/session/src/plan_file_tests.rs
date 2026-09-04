use super::plan_file;
use closeclaw_config::IdentifierFormat;
use std::path::Path;

#[test]
fn test_generate_identifier_timestamp_format() {
    let id = plan_file::generate_identifier("my feature", IdentifierFormat::Timestamp);
    // Format: yyyy-MM-dd-HH-mm-ss-slug
    assert!(id.starts_with("20"));
    assert!(id.contains('-'));
    let parts: Vec<&str> = id.splitn(7, '-').collect();
    assert!(
        parts.len() >= 6,
        "identifier should have at least 6 dash-separated parts, got: {id}"
    );
}

#[test]
fn test_generate_identifier_empty_title() {
    let id = plan_file::generate_identifier("", IdentifierFormat::Timestamp);
    assert!(
        id.ends_with("-untitled"),
        "empty title should end with -untitled, got: {id}"
    );
}

#[test]
fn test_generate_identifier_long_title_truncated() {
    let long_title = "a".repeat(100);
    let id = plan_file::generate_identifier(&long_title, IdentifierFormat::Timestamp);
    let parts: Vec<&str> = id.splitn(7, '-').collect();
    let slug = parts.last().unwrap_or(&"");
    assert!(
        slug.len() <= 50,
        "slug should be at most 50 chars, got {} chars: {}",
        slug.len(),
        slug
    );
}

#[test]
fn test_generate_identifier_special_chars() {
    let id = plan_file::generate_identifier("Hello World! @#$%", IdentifierFormat::Timestamp);
    let parts: Vec<&str> = id.splitn(7, '-').collect();
    let slug = parts.last().unwrap_or(&"");
    // Special chars replaced with hyphens, collapsed
    assert!(!slug.contains('!'));
    assert!(!slug.contains('@'));
}

#[test]
fn test_create_plan_file_normal() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "User Auth Flow").unwrap();

    assert!(path.exists(), "plan file should exist at {path:?}");
    assert!(path.starts_with(dir.path().join("plans")));

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("# User Auth Flow"),
        "file should contain title"
    );
    assert!(
        content.contains("## Context"),
        "file should contain Context section"
    );
    assert!(
        content.contains("## Tasks"),
        "file should contain Tasks section"
    );
    assert!(
        content.contains("## Verification"),
        "file should contain Verification section"
    );
    assert!(
        content.contains("## Notes"),
        "file should contain Notes section"
    );
}

#[test]
fn test_create_plan_file_empty_title() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "").unwrap();

    assert!(
        path.exists(),
        "plan file should exist even with empty title"
    );

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("# "),
        "file should contain empty title heading"
    );
}

#[test]
fn test_create_plan_file_creates_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    // plans/ directory should not exist yet
    assert!(!dir.path().join("plans").exists());

    let path = plan_file::create_plan_file(dir.path(), "Test").unwrap();

    assert!(
        dir.path().join("plans").exists(),
        "plans directory should be created"
    );
    assert!(path.exists(), "plan file should be created");
}

#[test]
fn test_create_plan_file_long_title() {
    let dir = tempfile::TempDir::new().unwrap();
    let long_title =
        "Very Long Feature Name That Exceeds Fifty Characters And Should Be Handled Gracefully";
    let path = plan_file::create_plan_file(dir.path(), long_title).unwrap();

    assert!(path.exists());

    let content = std::fs::read_to_string(&path).unwrap();
    // Title should be preserved in full (only slug is truncated)
    assert!(content.contains(&format!("# {long_title}")));
}

#[test]
fn test_generate_identifier_different_titles() {
    let id_a = plan_file::generate_identifier("Feature A", IdentifierFormat::Timestamp);
    let id_b = plan_file::generate_identifier("Feature B", IdentifierFormat::Timestamp);
    assert_ne!(
        id_a, id_b,
        "different titles should produce different identifiers"
    );
}

#[test]
fn test_create_plan_file_unique_identifiers() {
    let dir = tempfile::TempDir::new().unwrap();
    let path1 = plan_file::create_plan_file(dir.path(), "Feature A").unwrap();
    let path2 = plan_file::create_plan_file(dir.path(), "Feature B").unwrap();

    assert_ne!(
        path1, path2,
        "two plan files should have different identifiers"
    );
}

// ── Random Words Format Tests ───────────────────────────────────────────

#[test]
fn test_generate_random_identifier_format() {
    let id = plan_file::generate_random_identifier();
    // Format: {adjective}-{noun}-{noun}
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(
        parts.len(),
        3,
        "random identifier should have exactly 3 dash-separated parts, got: {id}"
    );
}

#[test]
fn test_generate_random_identifier_uses_valid_words() {
    // Generate many identifiers and verify all words are from known lists
    for _ in 0..50 {
        let id = plan_file::generate_random_identifier();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3, "should have 3 parts: {id}");
        // We can't check exact word lists here (private), but we can check
        // the format is lowercase alphanumeric with hyphens
        assert!(
            id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "identifier should be lowercase with hyphens: {id}"
        );
    }
}

#[test]
fn test_generate_random_identifier_uniqueness() {
    // Generate many identifiers - at least some should be different
    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
        ids.insert(plan_file::generate_random_identifier());
    }
    assert!(
        ids.len() > 1,
        "should generate different identifiers, got {} unique out of 100",
        ids.len()
    );
}

#[test]
fn test_generate_identifier_random_words_format() {
    let id = plan_file::generate_identifier("ignored", IdentifierFormat::RandomWords);
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(
        parts.len(),
        3,
        "random format should have 3 parts, got: {id}"
    );
}

#[test]
fn test_create_plan_file_with_format_timestamp() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file_with_format(
        dir.path(),
        "Test Feature",
        IdentifierFormat::Timestamp,
    )
    .unwrap();
    assert!(path.exists());
    // Filename should start with year
    let filename = path.file_stem().unwrap().to_str().unwrap();
    assert!(
        filename.starts_with("20"),
        "timestamp id should start with year: {filename}"
    );
}

#[test]
fn test_create_plan_file_with_format_random_words() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file_with_format(
        dir.path(),
        "Test Feature",
        IdentifierFormat::RandomWords,
    )
    .unwrap();
    assert!(path.exists());
    // Filename should be adjective-noun-noun
    let filename = path.file_stem().unwrap().to_str().unwrap();
    let parts: Vec<&str> = filename.split('-').collect();
    assert_eq!(
        parts.len(),
        3,
        "random format filename should have 3 parts: {filename}"
    );
}

#[test]
fn test_create_plan_file_with_format_default_is_timestamp() {
    // create_plan_file (no format) should default to timestamp
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Default Format").unwrap();
    let filename = path.file_stem().unwrap().to_str().unwrap();
    assert!(
        filename.starts_with("20"),
        "default should be timestamp: {filename}"
    );
}

// ── update_plan_timestamp tests ──────────────────────────────────────────

#[test]
fn test_update_plan_timestamp_normal() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Test").unwrap();
    // Seed a known distinct timestamp to verify replacement without sleep
    let seed_ts = "0000-00-00 00:00:00";
    let content = std::fs::read_to_string(&path).unwrap();
    let seeded = content.replace(
        content.lines().find(|l| l.contains("更新时间")).unwrap(),
        &format!("| 更新时间 | {seed_ts} |"),
    );
    std::fs::write(&path, &seeded).unwrap();

    let result = plan_file::update_plan_timestamp(path.to_str().unwrap());
    assert!(result.is_ok(), "update_plan_timestamp should succeed");

    let updated = std::fs::read_to_string(&path).unwrap();
    // Verify timestamp was replaced (no longer the seeded value)
    let updated_ts = updated.lines().find(|l| l.contains("更新时间")).unwrap();
    assert_ne!(
        updated_ts,
        format!("| 更新时间 | {seed_ts} |"),
        "timestamp should be replaced"
    );
    assert!(
        updated_ts.contains("| 更新时间 | "),
        "should still be a timestamp line"
    );
}

#[test]
fn test_update_plan_timestamp_file_not_found() {
    let result = plan_file::update_plan_timestamp("/nonexistent/path/plan.md");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn test_update_plan_timestamp_line_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("no-ts.md");
    std::fs::write(&path, "# Plan\n\nNo timestamp line.\n").unwrap();

    let result = plan_file::update_plan_timestamp(path.to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

// ── PLAN_TEMPLATE field tests ────────────────────────────────────────────

#[test]
fn test_plan_template_has_update_time_field() {
    assert!(
        plan_file::PLAN_TEMPLATE.contains("更新时间"),
        "PLAN_TEMPLATE should contain 更新时间 field"
    );
}

#[test]
fn test_plan_template_has_create_time_field() {
    assert!(
        plan_file::PLAN_TEMPLATE.contains("创建时间"),
        "PLAN_TEMPLATE should contain 创建时间 field"
    );
}

#[test]
fn test_create_plan_file_fills_both_timestamps() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Test").unwrap();
    let content = std::fs::read_to_string(&path).unwrap();

    let create_lines: Vec<&str> = content.lines().filter(|l| l.contains("创建时间")).collect();
    let update_lines: Vec<&str> = content.lines().filter(|l| l.contains("更新时间")).collect();
    assert_eq!(
        create_lines.len(),
        1,
        "should have exactly one 创建时间 line"
    );
    assert_eq!(
        update_lines.len(),
        1,
        "should have exactly one 更新时间 line"
    );
    // Both should have a timestamp value (not just the placeholder)
    assert!(
        create_lines[0].contains("20"),
        "创建时间 should have year, got: {}",
        create_lines[0]
    );
    assert!(
        update_lines[0].contains("20"),
        "更新时间 should have year, got: {}",
        update_lines[0]
    );
}

// ── resolve_plan_by_name tests ──────────────────────────────────────────

/// Helper to create a plans directory with the given file stems.
fn setup_plans_dir(dir: &Path, stems: &[&str]) {
    let plans = dir.join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    for stem in stems {
        std::fs::write(plans.join(format!("{stem}.md")), "# Plan").unwrap();
    }
}

#[test]
fn test_resolve_exact_match() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha-feature", "beta-feature"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "alpha-feature");
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("alpha-feature.md"));
}

#[test]
fn test_resolve_exact_match_with_md_suffix() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["my-plan"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "my-plan.md");
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("my-plan.md"));
}

#[test]
fn test_resolve_prefix_match_unique() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["2026-08-19-04-31-design-doc"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "2026");
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("2026-08-19-04-31-design-doc.md"));
}

#[test]
fn test_resolve_prefix_match_ambiguous() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha-aaa", "alpha-bbb"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "alpha-");
    assert!(result.is_err());
    match result.unwrap_err() {
        plan_file::PlanResolveError::Ambiguous { name, candidates } => {
            assert_eq!(name, "alpha-");
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected Ambiguous, got: {other}"),
    }
}

#[test]
fn test_resolve_fuzzy_match_unique() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["2026-08-19-auth-feature"]);

    // Not exact, not prefix, but substring
    let result = plan_file::resolve_plan_by_name(dir.path(), "auth");
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("2026-08-19-auth-feature.md"));
}

#[test]
fn test_resolve_fuzzy_match_ambiguous() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["auth-login", "auth-logout"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "auth");
    assert!(result.is_err());
    match result.unwrap_err() {
        plan_file::PlanResolveError::Ambiguous { name, candidates } => {
            assert_eq!(name, "auth");
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected Ambiguous, got: {other}"),
    }
}

#[test]
fn test_resolve_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "gamma");
    assert!(result.is_err());
    match result.unwrap_err() {
        plan_file::PlanResolveError::NotFound { name } => {
            assert_eq!(name, "gamma");
        }
        other => panic!("expected NotFound, got: {other}"),
    }
}

#[test]
fn test_resolve_not_found_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    // plans/ directory does not exist

    let result = plan_file::resolve_plan_by_name(dir.path(), "anything");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        plan_file::PlanResolveError::NotFound { .. }
    ));
}

#[test]
fn test_resolve_not_found_empty_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();

    let result = plan_file::resolve_plan_by_name(dir.path(), "something");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        plan_file::PlanResolveError::NotFound { .. }
    ));
}

#[test]
fn test_resolve_ignores_non_md_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(plans.join("target.txt"), "text").unwrap();
    std::fs::write(plans.join("other.md"), "# Other").unwrap();

    let result = plan_file::resolve_plan_by_name(dir.path(), "target");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        plan_file::PlanResolveError::NotFound { .. }
    ));
}

#[test]
fn test_resolve_empty_query() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), "");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        plan_file::PlanResolveError::NotFound { .. }
    ));
}

#[test]
fn test_resolve_empty_query_after_md_strip() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha"]);

    let result = plan_file::resolve_plan_by_name(dir.path(), ".md");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        plan_file::PlanResolveError::NotFound { .. }
    ));
}

#[test]
fn test_resolve_exact_preferred_over_prefix() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_plans_dir(dir.path(), &["alpha", "alpha-beta"]);

    // "alpha" is exact for "alpha" but also prefix for "alpha-beta"
    // Exact match should win
    let result = plan_file::resolve_plan_by_name(dir.path(), "alpha");
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("alpha.md"));
}

#[test]
fn test_resolve_not_found_error_display() {
    let err = plan_file::PlanResolveError::NotFound {
        name: "test-plan".to_string(),
    };
    assert!(err.to_string().contains("test-plan"));
}

#[test]
fn test_resolve_ambiguous_error_display() {
    let err = plan_file::PlanResolveError::Ambiguous {
        name: "test".to_string(),
        candidates: vec!["test-a".to_string(), "test-b".to_string()],
    };
    let msg = err.to_string();
    assert!(msg.contains("test"));
    assert!(msg.contains("test-a"));
    assert!(msg.contains("test-b"));
}

// ── list_plan_summaries tests ──────────────────────────────────────────

/// Create a plan file with the given content.
fn create_plan_file_with_content(dir: &Path, stem: &str, content: &str) {
    let plans = dir.join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(plans.join(format!("{stem}.md")), content).unwrap();
}

/// Set the modification time of a file.
fn set_mtime(path: &Path, seconds_since_epoch: u64) {
    let time = filetime::FileTime::from_unix_time(seconds_since_epoch as i64, 0);
    filetime::set_file_mtime(path, time).unwrap();
}

#[test]
fn test_list_summaries_empty_when_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert!(summaries.is_empty());
}

#[test]
fn test_list_summaries_empty_when_no_files() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert!(summaries.is_empty());
}

#[test]
fn test_list_summaries_ignores_non_md_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(plans.join("notes.txt"), "hello").unwrap();
    std::fs::write(plans.join("plan.json"), "{}").unwrap();
    std::fs::write(plans.join("real.md"), "# Real Plan").unwrap();

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].stem, "real");
}

#[test]
fn test_list_summaries_sorted_by_mtime_desc() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(dir.path(), "old-plan", "# Old Plan");
    create_plan_file_with_content(dir.path(), "new-plan", "# New Plan");

    let old_path = dir.path().join("plans/old-plan.md");
    let new_path = dir.path().join("plans/new-plan.md");
    set_mtime(&old_path, 1000);
    set_mtime(&new_path, 2000);

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].stem, "new-plan");
    assert_eq!(summaries[1].stem, "old-plan");
}

#[test]
fn test_list_summaries_extracts_title() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(dir.path(), "plan-a", "# 用户认证功能\n\nSome content.");

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].title, "用户认证功能");
}

#[test]
fn test_list_summaries_counts_checkboxes() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-a",
        "# Plan A\n\n## Tasks\n\n- [x] Done task\n- [!] Important done\n- [~] In progress\n- [ ] Pending task\n- [ ] Another pending\n",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].completed, 1); // [x] only
    assert_eq!(summaries[0].failed, 1); // [!]
    assert_eq!(summaries[0].skipped, 1); // [~]
    assert_eq!(summaries[0].total, 5); // all - [ lines
}

#[test]
fn test_list_summaries_no_tasks_section_zero_counts() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-b",
        "# Plan B\n\n## Context\n\nSome context.",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].completed, 0);
    assert_eq!(summaries[0].failed, 0);
    assert_eq!(summaries[0].skipped, 0);
    assert_eq!(summaries[0].total, 0);
}

#[test]
fn test_list_summaries_tasks_no_checkboxes() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-c",
        "# Plan C\n\n## Tasks\n\nSome notes here\n- Item without checkbox\n",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].completed, 0);
    assert_eq!(summaries[0].failed, 0);
    assert_eq!(summaries[0].skipped, 0);
    assert_eq!(summaries[0].total, 0);
}

#[test]
fn test_list_summaries_chinese_title() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(dir.path(), "plan-zh", "# 实现用户认证功能");

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].title, "实现用户认证功能");
}

#[test]
fn test_list_summaries_only_completed_checkboxes() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-done",
        "# Plan Done\n\n## Tasks\n\n- [x] Task 1\n- [x] Task 2\n- [x] Task 3\n",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].completed, 3);
    assert_eq!(summaries[0].failed, 0);
    assert_eq!(summaries[0].skipped, 0);
    assert_eq!(summaries[0].total, 3);
}

#[test]
fn test_list_summaries_only_pending_checkboxes() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-pending",
        "# Plan Pending\n\n## Tasks\n\n- [ ] Task A\n- [ ] Task B\n",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries[0].completed, 0);
    assert_eq!(summaries[0].failed, 0);
    assert_eq!(summaries[0].skipped, 0);
    assert_eq!(summaries[0].total, 2);
}

#[test]
fn test_list_summaries_multiple_plans_mixed_states() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-1",
        "# Plan 1\n\n## Tasks\n\n- [x] Done\n- [ ] Pending\n",
    );
    create_plan_file_with_content(
        dir.path(),
        "plan-2",
        "# Plan 2\n\n## Tasks\n\n- [~] WIP\n- [!] Critical\n",
    );
    create_plan_file_with_content(dir.path(), "plan-3", "# Plan 3\n");

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    assert_eq!(summaries.len(), 3);

    let plan1 = summaries.iter().find(|s| s.stem == "plan-1").unwrap();
    assert_eq!(plan1.completed, 1);
    assert_eq!(plan1.failed, 0);
    assert_eq!(plan1.skipped, 0);
    assert_eq!(plan1.total, 2);

    let plan2 = summaries.iter().find(|s| s.stem == "plan-2").unwrap();
    assert_eq!(plan2.completed, 0);
    assert_eq!(plan2.failed, 1);
    assert_eq!(plan2.skipped, 1);
    assert_eq!(plan2.total, 2);

    let plan3 = summaries.iter().find(|s| s.stem == "plan-3").unwrap();
    assert_eq!(plan3.completed, 0);
    assert_eq!(plan3.failed, 0);
    assert_eq!(plan3.skipped, 0);
    assert_eq!(plan3.total, 0);
}

#[test]
fn test_list_summaries_stops_at_next_section() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file_with_content(
        dir.path(),
        "plan-sec",
        "# Plan Sec\n\n## Tasks\n\n- [x] Done\n\n## Verification\n\n- [ ] Verify\n",
    );

    let summaries = plan_file::list_plan_summaries(dir.path()).unwrap();
    // Only the task in the Tasks section counts
    assert_eq!(summaries[0].completed, 1);
    assert_eq!(summaries[0].failed, 0);
    assert_eq!(summaries[0].skipped, 0);
    assert_eq!(summaries[0].total, 1);
}

#[test]
fn test_read_plan_content_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("test.md");
    std::fs::write(&path, "# Hello\n\nContent here.").unwrap();

    let content = plan_file::read_plan_content(&path).unwrap();
    assert_eq!(content, "# Hello\n\nContent here.");
}

#[test]
fn test_read_plan_content_not_found() {
    let result = plan_file::read_plan_content(Path::new("/nonexistent/plan.md"));
    assert!(result.is_err());
}

#[test]
fn test_read_plan_content_chinese_content() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("plan.md");
    std::fs::write(&path, "# 中文标题\n\n这是中文内容。").unwrap();

    let content = plan_file::read_plan_content(&path).unwrap();
    assert!(content.contains("中文标题"));
    assert!(content.contains("中文内容"));
}

// ── append_to_plan_section / read_plan_section tests ───────────────────

fn create_plan_with_sections(dir: &Path) -> std::path::PathBuf {
    let content = "# Test Plan\n\n| 字段 | 值 |\n|------|-----|\n| 创建时间 | 2026-01-01 00:00:00 |\n| 更新时间 | 2026-01-01 00:00:00 |\n\n## Context\n\nInitial context.\n\n## Tasks\n\n- [ ] Task 1\n\n## Verification\n\n## Notes\n\n";
    let plans = dir.join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    let path = plans.join("test-plan.md");
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn test_append_to_plan_section_existing_section() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    plan_file::append_to_plan_section(&path, "Context", "More context details.").unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("Initial context."));
    assert!(content.contains("More context details."));
    // Verify order: original before new
    let ctx_pos = content.find("Initial context.").unwrap();
    let new_pos = content.find("More context details.").unwrap();
    assert!(ctx_pos < new_pos, "new content should come after existing");
}

#[test]
fn test_append_to_plan_section_creates_new_section() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    plan_file::append_to_plan_section(&path, "CustomSection", "Custom content.").unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("## CustomSection"));
    assert!(content.contains("Custom content."));
}

#[test]
fn test_append_to_plan_section_preserves_other_sections() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    plan_file::append_to_plan_section(&path, "Context", "Added.").unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    // Tasks section should be unchanged
    assert!(content.contains("- [ ] Task 1"));
    assert!(content.contains("## Tasks"));
    assert!(content.contains("## Verification"));
    assert!(content.contains("## Notes"));
}

#[test]
fn test_append_to_plan_section_updates_timestamp() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    // Seed a known timestamp
    let content = std::fs::read_to_string(&path).unwrap();
    let seeded = content.replace(
        content.lines().find(|l| l.contains("更新时间")).unwrap(),
        "| 更新时间 | 0000-00-00 00:00:00 |",
    );
    std::fs::write(&path, &seeded).unwrap();

    plan_file::append_to_plan_section(&path, "Context", "New.").unwrap();

    let updated = std::fs::read_to_string(&path).unwrap();
    let ts_line = updated.lines().find(|l| l.contains("更新时间")).unwrap();
    assert_ne!(ts_line, "| 更新时间 | 0000-00-00 00:00:00 |");
    assert!(ts_line.contains("20"), "timestamp should be updated");
}

#[test]
fn test_read_plan_section_existing() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    let ctx = plan_file::read_plan_section(&path, "Context").unwrap();
    assert_eq!(ctx, "Initial context.");
}

#[test]
fn test_read_plan_section_nonexistent() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    let result = plan_file::read_plan_section(&path, "Nonexistent").unwrap();
    assert_eq!(result, "");
}

#[test]
fn test_read_plan_section_empty_section() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    // Verification section is empty in the template
    let result = plan_file::read_plan_section(&path, "Verification").unwrap();
    assert_eq!(result, "");
}

#[test]
fn test_read_plan_section_multiline() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = "# Plan\n\n## Tasks\n\n- [ ] Step 1\n- [ ] Step 2\n\n## Notes\n\nSome notes.\n";
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    let path = plans.join("plan.md");
    std::fs::write(&path, content).unwrap();

    let tasks = plan_file::read_plan_section(&path, "Tasks").unwrap();
    assert!(tasks.contains("- [ ] Step 1"));
    assert!(tasks.contains("- [ ] Step 2"));
}

#[test]
fn test_append_then_read_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_with_sections(dir.path());

    plan_file::append_to_plan_section(&path, "Context", "Roundtrip test.").unwrap();

    let ctx = plan_file::read_plan_section(&path, "Context").unwrap();
    assert!(ctx.contains("Initial context."));
    assert!(ctx.contains("Roundtrip test."));
}

#[test]
fn test_append_to_plan_section_not_found() {
    let result =
        plan_file::append_to_plan_section(Path::new("/nonexistent/plan.md"), "Context", "content");
    assert!(result.is_err());
}

#[test]
fn test_read_plan_section_not_found() {
    let result = plan_file::read_plan_section(Path::new("/nonexistent/plan.md"), "Context");
    assert!(result.is_err());
}
