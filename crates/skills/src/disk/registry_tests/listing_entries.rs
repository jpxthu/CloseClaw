//! Tests for `DiskSkillRegistry::listing_entries_with_names` and
//! `listing_entries_with_activated`.

use super::super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use super::super::super::DiskSkill;
use super::super::super::DiskSkillRegistry;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn skill(name: &str, source: SkillSource) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_conditional(name: &str, source: SkillSource, paths: Vec<String>) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths,
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_not_invocable(name: &str, source: SkillSource) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_not_invocable_conditional(
    name: &str,
    source: SkillSource,
    paths: Vec<String>,
) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths,
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

// ===================================================================
// listing_entries_with_names
// ===================================================================

// --- No whitelist ---

#[test]
fn test_listing_entries_no_whitelist_returns_all_user_invocable() {
    let r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
        skill_not_invocable("hidden", SkillSource::Agent),
    ]);
    let entries = r.listing_entries_with_names(None, false);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));
    assert!(!names.contains(&"hidden"));
}

#[test]
fn test_listing_entries_no_whitelist_empty_registry() {
    let r = DiskSkillRegistry::new(vec![]);
    let entries = r.listing_entries_with_names(None, false);
    assert!(entries.is_empty());
}

// --- Specific whitelist ---

#[test]
fn test_listing_entries_specific_whitelist_filters() {
    let r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
        skill("c", SkillSource::Agent),
    ]);
    let entries = r.listing_entries_with_names(Some(&["b".into(), "c".into()]), false);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(!names.contains(&"a"));
    assert!(names.contains(&"b"));
    assert!(names.contains(&"c"));
}

#[test]
fn test_listing_entries_specific_whitelist_empty_list() {
    let r = DiskSkillRegistry::new(vec![skill("a", SkillSource::Bundled)]);
    let entries = r.listing_entries_with_names(Some(&[]), false);
    assert!(entries.is_empty());
}

// --- Wildcard whitelist ---

#[test]
fn test_listing_entries_wildcard_whitelist_returns_all() {
    let r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
    ]);
    let entries = r.listing_entries_with_names(Some(&["*".into()]), false);
    assert_eq!(entries.len(), 2);
}

// --- exclude_conditional ---

#[test]
fn test_listing_entries_exclude_conditional_true() {
    let r = DiskSkillRegistry::new(vec![
        skill("plain", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Global, vec!["**/*.rs".into()]),
    ]);
    let entries = r.listing_entries_with_names(None, true);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"plain"));
    assert!(!names.contains(&"cond"));
}

#[test]
fn test_listing_entries_exclude_conditional_false_includes_conditional() {
    let r = DiskSkillRegistry::new(vec![
        skill("plain", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Global, vec!["**/*.rs".into()]),
    ]);
    let entries = r.listing_entries_with_names(None, false);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"plain"));
    assert!(names.contains(&"cond"));
}

// --- Sorting ---

#[test]
fn test_listing_entries_sorted_by_source_then_name() {
    let r = DiskSkillRegistry::new(vec![
        skill("z_bundled", SkillSource::Bundled),
        skill("a_bundled", SkillSource::Bundled),
        skill("z_global", SkillSource::Global),
        skill("a_global", SkillSource::Global),
        skill("z_agent", SkillSource::Agent),
        skill("a_agent", SkillSource::Agent),
    ]);
    let entries = r.listing_entries_with_names(None, false);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    // Agent (highest priority) < Global < Bundled (lowest)
    assert_eq!(names[0], "a_agent");
    assert_eq!(names[1], "z_agent");
    assert_eq!(names[2], "a_global");
    assert_eq!(names[3], "z_global");
    assert_eq!(names[4], "a_bundled");
    assert_eq!(names[5], "z_bundled");
}

// --- Line content ---

#[test]
fn test_listing_entries_line_contains_description() {
    let r = DiskSkillRegistry::new(vec![skill("my_skill", SkillSource::Bundled)]);
    let entries = r.listing_entries_with_names(None, false);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].2.contains("**my_skill**"));
    assert!(entries[0].2.contains("desc of my_skill"));
}

// --- Source carried ---

#[test]
fn test_listing_entries_source_matches_disk_source() {
    let r = DiskSkillRegistry::new(vec![
        skill("proj", SkillSource::Project),
        skill("glo", SkillSource::Global),
    ]);
    let entries = r.listing_entries_with_names(None, false);
    let map: std::collections::HashMap<&str, SkillSource> =
        entries.iter().map(|(n, s, _)| (n.as_str(), *s)).collect();
    assert_eq!(map["proj"], SkillSource::Project);
    assert_eq!(map["glo"], SkillSource::Global);
}

// ===================================================================
// listing_entries_with_activated
// ===================================================================

// --- Activated conditional included ---

#[test]
fn test_activated_entries_includes_activated_conditional() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let entries = r.listing_entries_with_activated(None, &["cond".into()]);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"base"));
    assert!(names.contains(&"cond"));
}

#[test]
fn test_activated_entries_excludes_unactivated_conditional() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let entries = r.listing_entries_with_activated(None, &[]);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"base"));
    assert!(!names.contains(&"cond"));
}

// --- Activated conditional not invocable ---

#[test]
fn test_activated_entries_not_invocable_conditional_included() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_not_invocable_conditional(
            "hidden_cond",
            SkillSource::Bundled,
            vec!["**/*.rs".into()],
        ),
    ]);
    let entries = r.listing_entries_with_activated(None, &["hidden_cond".into()]);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"hidden_cond"));
}

// --- Non-conditional not invocable NOT exempt ---

#[test]
fn test_activated_entries_non_conditional_not_invocable_excluded() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_not_invocable("hidden_plain", SkillSource::Bundled),
    ]);
    let entries = r.listing_entries_with_activated(None, &["hidden_plain".into()]);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"base"));
    assert!(!names.contains(&"hidden_plain"));
}

// --- Whitelist interaction ---

#[test]
fn test_activated_entries_respects_whitelist() {
    let r = DiskSkillRegistry::new(vec![
        skill("alpha", SkillSource::Bundled),
        skill_conditional("beta_cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill("gamma", SkillSource::Bundled),
    ]);
    let entries = r.listing_entries_with_activated(
        Some(&["alpha".into(), "beta_cond".into()]),
        &["beta_cond".into()],
    );
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta_cond"));
    assert!(!names.contains(&"gamma"));
}

#[test]
fn test_activated_entries_wildcard_whitelist() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let entries = r.listing_entries_with_activated(Some(&["*".into()]), &["cond".into()]);
    assert_eq!(entries.len(), 2);
}

// --- Sorting ---

#[test]
fn test_activated_entries_sorted_by_source_then_name() {
    let r = DiskSkillRegistry::new(vec![
        skill("z_bundled", SkillSource::Bundled),
        skill_conditional("a_project", SkillSource::Project, vec!["**/*.rs".into()]),
        skill("b_global", SkillSource::Global),
    ]);
    let entries = r.listing_entries_with_activated(None, &["a_project".into()]);
    let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(names[0], "a_project"); // Project
    assert_eq!(names[1], "b_global"); // Global
    assert_eq!(names[2], "z_bundled"); // Bundled
}

// --- Line contains auto-activates annotation ---

#[test]
fn test_activated_entries_line_contains_annotation() {
    let r = DiskSkillRegistry::new(vec![skill_conditional(
        "rs_skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    let entries = r.listing_entries_with_activated(None, &["rs_skill".into()]);
    assert!(entries[0].2.contains("auto-activates on: **/*.rs"));
}

// --- Real source carried (not hardcoded) ---

#[test]
fn test_activated_entries_real_source_carried() {
    let r = DiskSkillRegistry::new(vec![skill_conditional(
        "cond",
        SkillSource::Agent,
        vec!["**/*.rs".into()],
    )]);
    let entries = r.listing_entries_with_activated(None, &["cond".into()]);
    assert_eq!(entries[0].1, SkillSource::Agent);
}

// --- Empty registry ---

#[test]
fn test_activated_entries_empty_registry() {
    let r = DiskSkillRegistry::new(vec![]);
    let entries = r.listing_entries_with_activated(None, &["any".into()]);
    assert!(entries.is_empty());
}
