use super::plan_file;

#[test]
fn test_generate_identifier_format() {
    let id = plan_file::generate_identifier("my feature");
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
    let id = plan_file::generate_identifier("");
    assert!(
        id.ends_with("-untitled"),
        "empty title should end with -untitled, got: {id}"
    );
}

#[test]
fn test_generate_identifier_long_title_truncated() {
    let long_title = "a".repeat(100);
    let id = plan_file::generate_identifier(&long_title);
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
    let id = plan_file::generate_identifier("Hello World! @#$%");
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
    let id_a = plan_file::generate_identifier("Feature A");
    let id_b = plan_file::generate_identifier("Feature B");
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
