use super::plan_file;
use closeclaw_config::IdentifierFormat;

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
        content.contains("draft"),
        "file should contain draft status"
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
        content.contains("draft"),
        "file should contain draft status"
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
