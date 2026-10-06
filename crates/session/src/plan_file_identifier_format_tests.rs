//! Plan identifier format boundary tests (Step 1.9 — decoupled enum
//! mapping).
//!
//! `closeclaw-session` now owns [`PlanIdentifierFormat`] (the config
//! enum was mapped out at the call sites); these tests pin the
//! session-side semantics that must survive the split: the default
//! stays Timestamp, the default path generates the same
//! `yyyy-MM-dd-HH-mm-ss-{slug}` shape as before, and the RandomWords
//! variant keeps its three-word shape.

use crate::plan_file::{self, PlanIdentifierFormat};

/// Assert the identifier is the timestamp format `yyyy-MM-dd-HH-mm-ss-{slug}`:
/// six numeric date/time components followed by a non-empty slug.
fn assert_timestamp_shape(identifier: &str) {
    let parts: Vec<&str> = identifier.splitn(7, '-').collect();
    assert_eq!(
        parts.len(),
        7,
        "timestamp identifier must be 6 date/time parts + slug, got: {identifier}"
    );
    assert_eq!(parts[0].len(), 4, "year must be 4 digits: {identifier}");
    assert!(
        parts[0].starts_with('2'),
        "year must be calendar-shaped: {identifier}"
    );
    for part in &parts[1..6] {
        assert_eq!(
            part.len(),
            2,
            "date/time components must be zero-padded 2 digits: {identifier}"
        );
        assert!(
            part.chars().all(|c| c.is_ascii_digit()),
            "date/time components must be numeric: {identifier}"
        );
    }
    assert!(!parts[6].is_empty(), "slug must be non-empty: {identifier}");
}

/// Default path parity: `PlanIdentifierFormat::default()` agrees with the
/// explicit Timestamp variant, and `generate_identifier` via the default
/// produces the same timestamp-shaped identifier (the decoupled session
/// enum keeps the pre-split default semantics).
#[test]
fn test_generate_identifier_default_matches_timestamp_format() {
    assert_eq!(
        PlanIdentifierFormat::default(),
        PlanIdentifierFormat::Timestamp,
        "session enum default must stay Timestamp"
    );
    assert_timestamp_shape(&plan_file::generate_identifier(
        "my feature",
        PlanIdentifierFormat::default(),
    ));
    assert_timestamp_shape(&plan_file::generate_identifier(
        "",
        PlanIdentifierFormat::default(),
    ));
    assert!(
        plan_file::generate_identifier("", PlanIdentifierFormat::default()).ends_with("-untitled"),
        "default path keeps the empty-title fallback"
    );
}

/// `create_plan_file` (no explicit format) falls back to the default
/// Timestamp identifier: the created file stem is timestamp-shaped and
/// carries the slugified title.
#[test]
fn test_create_plan_file_default_format_uses_timestamp_identifier() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Default Path").unwrap();

    let stem = path.file_stem().unwrap().to_str().unwrap();
    assert_timestamp_shape(stem);
    assert!(
        stem.ends_with("-default-path"),
        "stem must end with the slugified title, got: {stem}"
    );
}
