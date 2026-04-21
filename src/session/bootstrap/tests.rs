//! Tests for bootstrap protection

use chrono::Utc;

use super::*;

fn make_test_protection() -> BootstrapProtection {
    BootstrapProtection::new()
}

#[test]
fn test_bootstrap_region_new() {
    let content = "# AGENTS\n\nDo this and that.";
    let region = BootstrapRegion::new("AGENTS.md", content, false);

    assert_eq!(region.file_name, "AGENTS.md");
    assert!(!region.is_reinject);
    assert_eq!(region.char_count, content.chars().count());
    assert!(!region.region_id.is_empty());
}

#[test]
fn test_bootstrap_region_hash_integrity() {
    let content = "# AGENTS\n\nDo this and that.";
    let region = BootstrapRegion::new("AGENTS.md", content, false);

    assert!(region.verify_integrity(content));
    assert!(!region.verify_integrity("# Modified content"));
    assert!(!region.verify_integrity(""));
}

#[test]
fn test_bootstrap_region_wrap_content() {
    let content = "# AGENTS\n\nDo this and that.";
    let region = BootstrapRegion::new("AGENTS.md", content, false);

    let wrapped = region.wrap_content(content);
    assert!(wrapped.starts_with(BOOTSTRAP_REGION_START));
    assert!(wrapped.contains(content));
    assert!(wrapped.ends_with(BOOTSTRAP_REGION_END));
}

#[test]
fn test_bootstrap_region_parse_from_marker() {
    let content = "# AGENTS\n\nDo this and that.";
    let hash = BootstrapRegion::compute_hash(content);
    let marker = format!(
        "{}file=AGENTS.md,hash={},chars={},reinject=false>",
        BOOTSTRAP_REGION_START,
        &hash[..12],
        content.chars().count()
    );

    let region = BootstrapRegion::parse_from_marker(&marker, content, Utc::now()).unwrap();
    assert_eq!(region.file_name, "AGENTS.md");
    assert!(!region.is_reinject);
}

#[test]
fn test_bootstrap_context_default() {
    let ctx = BootstrapContext::default();
    assert!(ctx.regions.is_empty());
    assert!(ctx.reinjected_after_last_compact);
    assert_eq!(ctx.total_char_count, 0);
    assert!(ctx.pre_compact_hashes.is_empty());
}

#[test]
fn test_bootstrap_context_add_region() {
    let mut ctx = BootstrapContext::default();
    let region = BootstrapRegion::new("AGENTS.md", "# AGENTS", false);
    let char_count = region.char_count;

    ctx.add_region(region);
    assert_eq!(ctx.regions.len(), 1);
    assert_eq!(ctx.total_char_count, char_count);
}

#[test]
fn test_bootstrap_context_check_integrity() {
    let mut ctx = BootstrapContext::default();
    ctx.add_region(BootstrapRegion::new("AGENTS.md", "# AGENTS content", false));

    // Content matches - no corruption
    let corrupted = ctx.check_integrity([("AGENTS.md", "# AGENTS content")].into_iter());
    assert!(corrupted.is_empty());

    // Content modified - detected as corrupted
    let corrupted = ctx.check_integrity([("AGENTS.md", "# MODIFIED content")].into_iter());
    assert_eq!(corrupted.len(), 1);
    assert_eq!(corrupted[0], "AGENTS.md");
}

#[test]
fn test_bootstrap_context_exceeds_size_limit() {
    let mut ctx = BootstrapContext::default();
    ctx.total_char_count = 70 * 1024; // 70K

    assert!(ctx.exceeds_size_limit(60 * 1024));
    assert!(!ctx.exceeds_size_limit(80 * 1024));
}

#[test]
fn test_bootstrap_protection_before_compact() {
    let protection = make_test_protection();
    let mut ctx = BootstrapContext::default();
    ctx.add_region(BootstrapRegion::new("AGENTS.md", "# test", false));

    protection.before_compact(&mut ctx);

    assert_eq!(ctx.pre_compact_hashes.len(), 1);
    assert!(ctx
        .pre_compact_hashes
        .contains_key(&ctx.regions[0].region_id));
}

#[test]
fn test_bootstrap_protection_after_compact_no_corruption() {
    let protection = make_test_protection();
    let mut ctx = BootstrapContext::default();
    let content = "# AGENTS\n\nTest content.";
    ctx.add_region(BootstrapRegion::new("AGENTS.md", content, false));

    // Store pre-compact hashes
    protection.before_compact(&mut ctx);

    // Transcript unchanged - no corruption
    let wrapped = ctx.regions[0].wrap_content(content);
    let to_reinject = protection.after_compact(&wrapped, &mut ctx);

    assert!(
        to_reinject.is_empty(),
        "unexpected reinject needed: {:?}",
        to_reinject
    );
    assert!(ctx.reinjected_after_last_compact);
}

#[test]
fn test_bootstrap_protection_after_compact_with_corruption() {
    let protection = make_test_protection();
    let mut ctx = BootstrapContext::default();
    let original = "# AGENTS\n\nOriginal content.";
    ctx.add_region(BootstrapRegion::new("AGENTS.md", original, false));

    // Store pre-compact hashes
    protection.before_compact(&mut ctx);

    // Transcript modified by compaction
    let corrupted = "# SUMMARY: agent lost original context";
    let wrapped = ctx.regions[0].wrap_content(corrupted);
    let to_reinject = protection.after_compact(&wrapped, &mut ctx);

    assert_eq!(to_reinject.len(), 1);
    assert_eq!(to_reinject[0], "AGENTS.md");
    assert!(!ctx.reinjected_after_last_compact);
}

#[test]
fn test_bootstrap_protection_reinject() {
    let temp_dir = std::env::temp_dir();
    let protection = BootstrapProtection::new()
        .with_workspace(temp_dir.clone())
        .with_bootstrap_files(vec!["AGENTS.md".to_string()]);

    // Create a test bootstrap file
    let test_file = temp_dir.join("AGENTS.md");
    std::fs::write(&test_file, "# AGENTS\n\nTest content.").unwrap();

    let mut ctx = BootstrapContext::default();
    let result = protection.reinject(&["AGENTS.md".to_string()], &mut ctx);

    assert!(result.is_ok());
    let reinject_text = result.unwrap();
    assert!(reinject_text.contains(BOOTSTRAP_REGION_START));
    assert!(reinject_text.contains("# AGENTS"));
    assert!(reinject_text.contains(BOOTSTRAP_REGION_END));
    assert!(ctx.regions[0].is_reinject);

    // Cleanup
    std::fs::remove_file(test_file).ok();
}

#[test]
fn test_make_bootstrap_marker() {
    use super::helpers::make_bootstrap_marker;

    let content = "# AGENTS";
    let marker = make_bootstrap_marker("AGENTS.md", content, false);

    assert!(marker.starts_with(BOOTSTRAP_REGION_START));
    assert!(marker.contains("file=AGENTS.md"));
    assert!(marker.contains("reinject=false"));
    assert!(marker.ends_with('>'));
}

#[test]
fn test_region_id_unique() {
    let r1 = BootstrapRegion::new("AGENTS.md", "# Content A", false);
    let r2 = BootstrapRegion::new("AGENTS.md", "# Content B", false);

    // Same file with different content should have different region_ids
    // (because hash is part of region_id)
    assert_ne!(r1.region_id, r2.region_id);
}
